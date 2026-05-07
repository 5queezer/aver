use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use memory_server::{config::ServerConfig, http::build_router};
use tower::ServiceExt;

#[tokio::test]
async fn oauth_metadata_route_returns_discovery_document() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "https://aml.example.com".to_string(),
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
    assert_eq!(json["issuer"], "https://aml.example.com");
}
