use std::net::SocketAddr;

use aver_server::{
    auth::AuthDb, config::ServerConfig, http::build_router, oauth::pkce_s256_challenge,
};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

#[tokio::test]
async fn oauth_metadata_route_returns_discovery_document() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: dir.path().join("auth.db").to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
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
    assert_eq!(json["issuer"], "https://aver.example.com");
}

#[tokio::test]
async fn oauth_token_route_exchanges_authorization_code_with_pkce() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &pkce_s256_challenge(verifier),
            "http://localhost:8080/callback",
            &[],
        )
        .unwrap();
    drop(db);

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();
    let redirect = "http://localhost:8080/callback";
    let body = format!(
        "grant_type=authorization_code&code={code}&client_id=client-1&code_verifier={verifier}&redirect_uri={redirect}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["token_type"], "Bearer");
    assert!(json["access_token"].as_str().unwrap().len() > 10);
}

#[tokio::test]
async fn protected_health_requires_bearer_token() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    db.store_access_token_hash(
        &aver_server::auth::hash_token("secret-token"),
        "user-1",
        &[],
    )
    .unwrap();
    drop(db);

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header(header::AUTHORIZATION, "Bearer secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
}

#[tokio::test]
async fn oauth_register_route_creates_public_client() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: dir.path().join("auth.db").to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/oauth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "client_name": "Claude Desktop",
                        "redirect_uris": ["http://127.0.0.1:3917/callback"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["client_id"].as_str().unwrap().len() > 10);
    assert_eq!(json["client_name"], "Claude Desktop");
    assert_eq!(json["token_endpoint_auth_method"], "none");
}

#[tokio::test]
async fn oauth_authorize_route_rejects_non_loopback_with_html_403() {
    // ADR-0020 slice 6: only loopback callers can drive the consent flow.
    // Public-internet peers must see a terminal HTML 403 — never an OAuth
    // redirect-error that could bounce an attacker-supplied redirect_uri.
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    let client = db
        .register_client(
            "Claude Desktop",
            &["http://127.0.0.1:3917/callback".to_string()],
        )
        .unwrap();
    drop(db);
    let challenge = pkce_s256_challenge("verifier");

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={}&code_challenge_method=S256",
        client.client_id, challenge
    );

    // Non-loopback peer: explicitly inject a routable ConnectInfo so the
    // dispatcher classifies the call as public-internet.
    let routable: SocketAddr = "203.0.113.7:51234".parse().unwrap();
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request.extensions_mut().insert(ConnectInfo(routable));

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/html"),
        "expected HTML 403, got {content_type}",
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(
        text.contains("Authorization unavailable"),
        "expected non-loopback rejection page, got: {text}",
    );
}

#[tokio::test]
async fn oauth_authorize_route_rejects_missing_connect_info() {
    // Defensive check: requests without ConnectInfo (the most common test
    // shape pre-slice-6) are conservatively classified as non-loopback.
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let challenge = pkce_s256_challenge("verifier");

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id=anything&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={challenge}&code_challenge_method=S256",
    );
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mcp_route_requires_bearer_token() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: dir.path().join("auth.db").to_string_lossy().to_string(),
        cors_origins: Vec::new(),
    };
    let app = build_router(config).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
