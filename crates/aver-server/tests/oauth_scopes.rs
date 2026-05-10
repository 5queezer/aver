//! Integration tests for ADR-0020 slice 3 (post-slice-6):
//!
//! - `scopes_supported` advertised in discovery metadata.
//! - Scoped access-token issuance via the consent flow and refresh grant.
//! - Per-tool scope enforcement on `/mcp` (insufficient_scope rejection).

use std::net::SocketAddr;

use aver_server::auth::AuthDb;
use aver_server::config::ServerConfig;
use aver_server::http::build_router;
use aver_server::oauth::pkce_s256_challenge;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use tower::ServiceExt;

const VERIFIER: &str = "verifier-abc-1234567890-xyz";
const REDIRECT: &str = "http://127.0.0.1:3917/callback";

fn loopback_addr() -> SocketAddr {
    "127.0.0.1:54321".parse().unwrap()
}

fn base_config(dir: &tempfile::TempDir, auth_db_path: &std::path::Path) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "http://127.0.0.1:3317".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    }
}

fn register_client(auth_db_path: &std::path::Path) -> String {
    let db = AuthDb::open(auth_db_path).unwrap();
    db.register_client("Test MCP client", &[REDIRECT.to_string()])
        .unwrap()
        .client_id
}

fn extract_session_cookie(headers: &header::HeaderMap) -> Option<String> {
    let raw = headers.get(header::SET_COOKIE)?.to_str().ok()?;
    raw.split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("aver_session=").map(str::to_string))
}

fn extract_csrf(html: &str) -> String {
    let needle = "name=\"csrf_token\" value=\"";
    let i = html.find(needle).unwrap();
    let rest = &html[i + needle.len()..];
    let end = rest.find('"').unwrap();
    rest[..end].to_string()
}

fn extract_code_from_location(location: &str) -> String {
    let url = url::Url::parse(location).unwrap();
    url.query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap()
}

/// Drives GET /oauth/authorize → POST /oauth/authorize/decision (approve)
/// → POST /oauth/token. Returns the issued access + refresh tokens.
async fn approve_and_mint_tokens(
    app: &axum::Router,
    client_id: &str,
    scope_param: Option<&str>,
) -> (String, String) {
    let challenge = pkce_s256_challenge(VERIFIER);
    let mut authorize_uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    if let Some(scope) = scope_param {
        authorize_uri.push_str(&format!(
            "&scope={}",
            url::form_urlencoded::byte_serialize(scope.as_bytes()).collect::<String>(),
        ));
    }

    let mut req = Request::builder()
        .uri(&authorize_uri)
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session_cookie = extract_session_cookie(response.headers()).unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csrf = extract_csrf(std::str::from_utf8(&body).unwrap());

    let mut form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&csrf_token={csrf}&decision=approve",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    if let Some(scope) = scope_param {
        form.push_str(&format!(
            "&scope={}",
            url::form_urlencoded::byte_serialize(scope.as_bytes()).collect::<String>(),
        ));
    }
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code = extract_code_from_location(&location);

    let token_form = format!(
        "grant_type=authorization_code&code={code}&client_id={cid}&code_verifier={v}&redirect_uri={r}",
        cid = client_id,
        v = VERIFIER,
        r = REDIRECT,
    );
    let req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(token_form))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (
        json["access_token"].as_str().unwrap().to_string(),
        json["refresh_token"].as_str().unwrap().to_string(),
    )
}

/// Drives the full MCP handshake (initialize + initialized) then issues a
/// `tools/call` for `tool_name`. Returns the JSON-RPC response object.
async fn call_mcp_tool(
    app: &axum::Router,
    bearer: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    // 1) initialize: server creates a session, returns SSE response with
    //    Mcp-Session-Id header.
    let init_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"},
        },
    })
    .to_string();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::HOST, "127.0.0.1")
        .body(Body::from(init_body))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "initialize did not return 200",
    );
    let session_id = response
        .headers()
        .get("Mcp-Session-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    // Drain init body so the SSE worker completes.
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

    // 2) notifications/initialized: tells the server the client is ready.
    let notif_body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    })
    .to_string();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::HOST, "127.0.0.1")
        .header("Mcp-Session-Id", &session_id)
        .header("MCP-Protocol-Version", "2025-06-18")
        .body(Body::from(notif_body))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert!(
        response.status().is_success(),
        "initialized notification failed: {}",
        response.status(),
    );
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

    // 3) tools/call: dispatch the actual scope-checked tool.
    let call_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": tool_name, "arguments": args},
    })
    .to_string();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::HOST, "127.0.0.1")
        .header("Mcp-Session-Id", &session_id)
        .header("MCP-Protocol-Version", "2025-06-18")
        .body(Body::from(call_body))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    parse_first_sse_message(&body)
}

/// Parses the first JSON-RPC message out of an SSE stream body. Skips
/// priming events whose `data:` line is empty (the rmcp server emits one
/// before the actual response when `sse_retry` is configured).
fn parse_first_sse_message(body: &[u8]) -> serde_json::Value {
    let text = std::str::from_utf8(body).unwrap();
    for line in text.lines() {
        let payload = match line.strip_prefix("data:") {
            Some(rest) => rest.trim_start(),
            None => continue,
        };
        if payload.is_empty() {
            continue;
        }
        return serde_json::from_str(payload)
            .unwrap_or_else(|err| panic!("invalid SSE data payload {payload:?}: {err}"));
    }
    panic!("no JSON-bearing `data:` line in SSE body: {text:?}");
}

// -- Tests ----------------------------------------------------------------

#[tokio::test]
async fn discovery_metadata_lists_six_canonical_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-authorization-server")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["scopes_supported"],
        serde_json::json!([
            "claims:read",
            "claims:write",
            "events:write",
            "candidates:manage",
            "observations:read",
            "observations:write",
        ]),
    );
}

#[tokio::test]
async fn approve_records_granted_scopes_on_consent_row_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let client_id = register_client(&auth_db_path);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let (access_token, _) =
        approve_and_mint_tokens(&app, &client_id, Some("claims:read claims:write")).await;

    // Consent row carries the approved scopes.
    let db = AuthDb::open(&auth_db_path).unwrap();
    let consent = db.get_consent("local", &client_id).unwrap().unwrap();
    assert_eq!(
        consent.granted_scopes,
        vec!["claims:read".to_string(), "claims:write".to_string()],
    );

    // Access token row carries them too.
    let token_hash = aver_server::auth::hash_token(&access_token);
    let (user_id, scopes_raw) = db.validate_access_token(&token_hash).unwrap().unwrap();
    assert_eq!(user_id, "local");
    assert_eq!(scopes_raw, "claims:read claims:write");
}

#[tokio::test]
async fn read_only_scope_allows_recall_but_rejects_writes() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let client_id = register_client(&auth_db_path);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let (token, _) = approve_and_mint_tokens(&app, &client_id, Some("claims:read")).await;

    // recall: in-scope.
    let resp = call_mcp_tool(
        &app,
        &token,
        "recall",
        serde_json::json!({"query": "hello"}),
    )
    .await;
    assert!(
        resp.get("result").is_some() && resp.get("error").is_none(),
        "recall should succeed under claims:read but got: {resp}",
    );

    // remember_claim: out of scope (claims:write).
    let resp = call_mcp_tool(
        &app,
        &token,
        "remember_claim",
        serde_json::json!({"subject": "s", "predicate": "p", "object": "o"}),
    )
    .await;
    let err = resp
        .get("error")
        .expect("scope-rejected call returns error");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("insufficient_scope"),
        "expected insufficient_scope error, got: {err}",
    );

    // record_observation: out of scope (observations:write).
    let resp = call_mcp_tool(
        &app,
        &token,
        "record_observation",
        serde_json::json!({
            "session_id": "s",
            "content": "c",
            "relevance": "low",
            "source_event_ids": [],
            "derivation": "d",
        }),
    )
    .await;
    let err = resp
        .get("error")
        .expect("scope-rejected call returns error");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("insufficient_scope"),
        "expected insufficient_scope error, got: {err}",
    );
}

#[tokio::test]
async fn read_and_write_scopes_allow_both_tools() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let client_id = register_client(&auth_db_path);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let (token, _) =
        approve_and_mint_tokens(&app, &client_id, Some("claims:read claims:write")).await;

    let resp = call_mcp_tool(
        &app,
        &token,
        "remember_claim",
        serde_json::json!({"subject": "alice", "predicate": "relates_to", "object": "bob"}),
    )
    .await;
    assert!(
        resp.get("result").is_some() && resp.get("error").is_none(),
        "remember_claim should succeed under claims:write but got: {resp}",
    );

    let resp = call_mcp_tool(
        &app,
        &token,
        "recall",
        serde_json::json!({"query": "alice"}),
    )
    .await;
    assert!(
        resp.get("result").is_some() && resp.get("error").is_none(),
        "recall should succeed under claims:read but got: {resp}",
    );
}

#[tokio::test]
async fn approve_with_no_scope_param_grants_no_access() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let client_id = register_client(&auth_db_path);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let (token, _) = approve_and_mint_tokens(&app, &client_id, None).await;

    // No scope was requested, so the token has no implicit access.
    let resp = call_mcp_tool(&app, &token, "recall", serde_json::json!({"query": "x"})).await;
    let err = resp
        .get("error")
        .expect("missing scope means no implicit access");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("insufficient_scope"),
    );
}

#[tokio::test]
async fn refresh_token_grant_preserves_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let client_id = register_client(&auth_db_path);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    let (_access, refresh) = approve_and_mint_tokens(&app, &client_id, Some("claims:read")).await;

    let body = format!("grant_type=refresh_token&refresh_token={refresh}");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_access = json["access_token"].as_str().unwrap();

    let db = AuthDb::open(&auth_db_path).unwrap();
    let (_, scopes_raw) = db
        .validate_access_token(&aver_server::auth::hash_token(new_access))
        .unwrap()
        .unwrap();
    assert_eq!(scopes_raw, "claims:read");

    // And the refreshed token still authorises recall.
    let resp = call_mcp_tool(
        &app,
        new_access,
        "recall",
        serde_json::json!({"query": "y"}),
    )
    .await;
    assert!(
        resp.get("error").is_none(),
        "refreshed token should still recall: {resp}",
    );
}

#[tokio::test]
async fn unknown_persisted_scope_is_ignored_but_valid_scopes_still_work() {
    // Mint a token row by hand whose granted_scopes column holds an unknown
    // token alongside a real one. The MCP layer must allow the real scope's
    // tools while ignoring the bogus token.
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    let access_token = "manually-minted-token";
    db.store_access_token_hash(
        &aver_server::auth::hash_token(access_token),
        "local",
        &["claims:read".to_string(), "totally:unknown".to_string()],
    )
    .unwrap();
    drop(db);

    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();

    // Valid scope still works.
    let resp = call_mcp_tool(
        &app,
        access_token,
        "recall",
        serde_json::json!({"query": "x"}),
    )
    .await;
    assert!(resp.get("error").is_none(), "recall failed: {resp}");

    // Unknown scope grants nothing.
    let resp = call_mcp_tool(
        &app,
        access_token,
        "remember_claim",
        serde_json::json!({"subject": "a", "predicate": "b", "object": "c"}),
    )
    .await;
    let err = resp.get("error").expect("scope mismatch should error");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("insufficient_scope"),
    );
}
