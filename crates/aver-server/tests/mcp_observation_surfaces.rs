use aver_core::{ObservationRelevance, Store};
use aver_server::{
    auth::{AuthDb, hash_token},
    config::ServerConfig,
    http::build_router,
};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

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

fn issue_access_token(auth_db_path: &std::path::Path, token: &str, scopes: &[&str]) {
    let db = AuthDb::open(auth_db_path).unwrap();
    let scope_list = scopes
        .iter()
        .map(|scope| scope.to_string())
        .collect::<Vec<_>>();
    db.store_access_token_hash(&hash_token(token), "local", &scope_list)
        .unwrap();
}

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

async fn call_mcp_tool(
    app: &axum::Router,
    bearer: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    // Minimal MCP init handshake to obtain a session id.
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
        "initialize did not return 200"
    );
    let session_id = response
        .headers()
        .get("Mcp-Session-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

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
    assert!(response.status().is_success());
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

    let call_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": args,
        }
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

fn text_payload(tool_response: &serde_json::Value) -> &str {
    tool_response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should include JSON/text payload")
}

fn json_payload(tool_response: &serde_json::Value) -> serde_json::Value {
    serde_json::from_str(text_payload(tool_response)).unwrap()
}

fn assert_scope_denied(tool_response: &serde_json::Value) {
    let err = tool_response
        .get("error")
        .expect("scope-rejected call returns error");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("insufficient_scope"),
        "expected insufficient_scope error, got {err}",
    );
}

#[tokio::test]
async fn mcp_observation_coverage_requires_observations_read_scope() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();

    let app = build_router(base_config(&dir, &auth_db_path)).unwrap();

    let read_token = "obs-read-token";
    issue_access_token(
        &auth_db_path,
        read_token,
        &["observations:read", "claims:read"],
    );

    let store = Store::open(dir.path().join("memory")).unwrap();
    let session_id = "obs-session";
    let first = store
        .record_event(session_id, "message", "first build", "test")
        .unwrap();
    let second = store
        .record_event(session_id, "message", "second build", "test")
        .unwrap();
    let third = store
        .record_event(session_id, "message", "third build", "test")
        .unwrap();
    let _ = store
        .record_observation(
            session_id,
            "event 1 and 3 were summarized",
            ObservationRelevance::High,
            &vec![first, third],
            "spec test",
        )
        .unwrap();

    let resp = call_mcp_tool(
        &app,
        read_token,
        "observation_coverage",
        serde_json::json!({"session_id": session_id}),
    )
    .await;
    let payload = json_payload(&resp);
    assert_eq!(
        payload["covered_event_ids"],
        serde_json::json!([first, third])
    );
    assert_eq!(payload["uncovered_event_ids"], serde_json::json!([second]));

    let read_only_token = "claims-only-token";
    issue_access_token(&auth_db_path, read_only_token, &["claims:read"]);
    let denied = call_mcp_tool(
        &app,
        read_only_token,
        "observation_coverage",
        serde_json::json!({"session_id": session_id}),
    )
    .await;
    assert_scope_denied(&denied);

    let events_write_token = "events-write-token";
    issue_access_token(&auth_db_path, events_write_token, &["events:write"]);
    let writes_ok = call_mcp_tool(
        &app,
        events_write_token,
        "record_event",
        serde_json::json!({
            "session_id": session_id,
            "kind": "log",
            "payload": "write from MCP scope",
        }),
    )
    .await;
    assert!(writes_ok.get("result").is_some() && writes_ok.get("error").is_none());

    let write_denied = call_mcp_tool(
        &app,
        read_only_token,
        "record_event",
        serde_json::json!({
            "session_id": session_id,
            "kind": "log",
            "payload": "forbidden",
        }),
    )
    .await;
    assert_scope_denied(&write_denied);

    let _ = second;
    let _ = third;
}

#[tokio::test]
async fn mcp_compaction_summary_reports_uncovered_ranges_and_requires_observations_read() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let app = build_router(base_config(&dir, &auth_db_path)).unwrap();

    let token = "summary-token";
    issue_access_token(&auth_db_path, token, &["observations:read", "claims:read"]);
    let events_scope_token = "events-scope";
    issue_access_token(&auth_db_path, events_scope_token, &["events:write"]);

    let store = Store::open(dir.path().join("memory")).unwrap();
    let session_id = "sum-session";
    let first = store
        .record_event(session_id, "message", "first", "cli")
        .unwrap();
    let second = store
        .record_event(session_id, "message", "second", "cli")
        .unwrap();
    let _ = first;
    let _ = store
        .record_observation(
            session_id,
            "summary gap probe",
            ObservationRelevance::High,
            &[second],
            "spec test",
        )
        .unwrap();

    let resp = call_mcp_tool(
        &app,
        token,
        "assemble_compaction_summary",
        serde_json::json!({"session_id": session_id}),
    )
    .await;
    let summary = text_payload(&resp);
    assert!(summary.contains("continuity is incomplete"));
    assert!(summary.contains("uncovered event ranges"));

    let denied = call_mcp_tool(
        &app,
        events_scope_token,
        "assemble_compaction_summary",
        serde_json::json!({"session_id": session_id}),
    )
    .await;
    assert_scope_denied(&denied);
}

#[tokio::test]
async fn mcp_recall_observation_surface_returns_audit_status_when_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let app = build_router(base_config(&dir, &auth_db_path)).unwrap();

    let token = "audit-token";
    issue_access_token(&auth_db_path, token, &["observations:read"]);

    let store = Store::open(dir.path().join("memory")).unwrap();
    let session_id = "audit-session";
    let event_a = store
        .record_event(session_id, "tool_result", "low signal", "runner")
        .unwrap();
    let event_b = store
        .record_event(session_id, "tool_result", "high signal", "runner")
        .unwrap();
    let low_id = store
        .record_observation(
            session_id,
            "low-priority observation",
            ObservationRelevance::Low,
            &[event_a],
            "audit test",
        )
        .unwrap();
    let _ = store
        .record_observation(
            session_id,
            "high-priority observation",
            ObservationRelevance::High,
            &[event_b],
            "audit test",
        )
        .unwrap();

    let pruned = store.prune_observations(session_id, 1).unwrap();
    assert_eq!(pruned, 1);

    let resp = call_mcp_tool(
        &app,
        token,
        "recall_observation",
        serde_json::json!({"observation_id": low_id}),
    )
    .await;
    let recalled = json_payload(&resp);
    assert_eq!(recalled["audit_status"], serde_json::json!("pruned"));
    assert_eq!(
        recalled["observation"]["id"],
        serde_json::Value::String(low_id.clone())
    );

    let _ = event_a;
    let _ = event_b;
}
