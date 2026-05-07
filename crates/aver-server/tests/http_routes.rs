use aver_server::{
    auth::AuthDb, config::ServerConfig, http::build_router, oauth::pkce_s256_challenge,
};
use axum::{
    body::Body,
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
        .store_authorization_code("client-1", "user-1", &pkce_s256_challenge(verifier))
        .unwrap();
    drop(db);

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
    };
    let app = build_router(config).unwrap();
    let body = format!(
        "grant_type=authorization_code&code={code}&client_id=client-1&code_verifier={verifier}"
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
    db.store_access_token_hash(&aver_server::auth::hash_token("secret-token"), "user-1")
        .unwrap();
    drop(db);

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
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
async fn oauth_authorize_route_redirects_with_pkce_code() {
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
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_s256_challenge(verifier);

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aver.example.com".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
    };
    let app = build_router(config).unwrap();
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={}&code_challenge_method=S256&state=abc",
        client.client_id, challenge
    );

    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("http://127.0.0.1:3917/callback?code="));
    assert!(location.contains("&state=abc"));
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
