//! The HTTP server opens ONE memory store at startup and shares it across
//! MCP sessions (per-session `Store::open` would break the single-writer
//! invariant on the SQLite database). These tests prove that two concurrent
//! sessions write into — and read from — the same store.

use aver_server::{auth::AuthDb, config::ServerConfig, http::build_router};
use axum::{
    body::Body,
    http::{Method, Request, header},
};
use tower::ServiceExt;

const BEARER: &str = "shared-store-test-token";

fn base_config(dir: &tempfile::TempDir, auth_db_path: &std::path::Path) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "http://127.0.0.1:3317".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
        trusted_auth_header: None,
    }
}

/// Parses the first JSON-RPC message out of an SSE stream body. Skips
/// priming events whose `data:` line is empty.
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

async fn mcp_post(app: &axum::Router, session_id: Option<&str>, body: String) -> (String, String) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {BEARER}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::HOST, "127.0.0.1");
    if let Some(id) = session_id {
        builder = builder
            .header("Mcp-Session-Id", id)
            .header("MCP-Protocol-Version", "2025-06-18");
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    // Notifications answer 202, requests 200 — both are fine here.
    assert!(
        response.status().is_success(),
        "MCP POST failed: {}",
        response.status(),
    );
    let returned_session = response
        .headers()
        .get("Mcp-Session-Id")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (returned_session, String::from_utf8(body.to_vec()).unwrap())
}

/// initialize + notifications/initialized; returns the session id.
async fn mcp_open_session(app: &axum::Router) -> String {
    let init = serde_json::json!({
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
    let (session_id, _) = mcp_post(app, None, init).await;
    assert!(!session_id.is_empty(), "initialize returns a session id");

    let notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    })
    .to_string();
    mcp_post(app, Some(&session_id), notif).await;
    session_id
}

async fn mcp_call_tool(
    app: &axum::Router,
    session_id: &str,
    tool: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": tool, "arguments": args},
    })
    .to_string();
    let (_, body) = mcp_post(app, Some(session_id), call).await;
    parse_first_sse_message(body.as_bytes())
}

#[tokio::test]
async fn two_concurrent_sessions_write_into_one_shared_store() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    db.store_access_token_hash(
        &aver_server::auth::hash_token(BEARER),
        "user-1",
        &["claims:read".to_string(), "claims:write".to_string()],
    )
    .unwrap();
    drop(db);

    let app = build_router(base_config(&dir, &auth_db_path)).unwrap();

    // Two live MCP sessions against the same server.
    let session_one = mcp_open_session(&app).await;
    let session_two = mcp_open_session(&app).await;
    assert_ne!(session_one, session_two);

    // Both sessions write a claim concurrently.
    let (result_one, result_two) = tokio::join!(
        mcp_call_tool(
            &app,
            &session_one,
            "remember_claim",
            serde_json::json!({
                "subject": "shared-marker-session-one",
                "predicate": "relates_to",
                "object": "shared-marker",
            }),
        ),
        mcp_call_tool(
            &app,
            &session_two,
            "remember_claim",
            serde_json::json!({
                "subject": "shared-marker-session-two",
                "predicate": "relates_to",
                "object": "shared-marker",
            }),
        ),
    );
    assert!(
        result_one.get("error").is_none(),
        "session one write failed: {result_one}",
    );
    assert!(
        result_two.get("error").is_none(),
        "session two write failed: {result_two}",
    );

    // A recall from session two sees BOTH claims: the sessions share one
    // store rather than each opening its own connection/database view.
    let recall = mcp_call_tool(
        &app,
        &session_two,
        "recall",
        serde_json::json!({"query": "shared-marker"}),
    )
    .await;
    let recall_text = serde_json::to_string(&recall).unwrap();
    assert!(
        recall_text.contains("shared-marker-session-one"),
        "session two must see session one's claim: {recall_text}",
    );
    assert!(
        recall_text.contains("shared-marker-session-two"),
        "session two must see its own claim: {recall_text}",
    );
}
