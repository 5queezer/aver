//! Integration tests for ADR-0020 slice 2: the loopback consent flow.

use std::net::SocketAddr;

use aver_server::auth::AuthDb;
use aver_server::config::ServerConfig;
use aver_server::consent::{CSRF_SECRET_NAME, compute_csrf_token};
use aver_server::http::build_router;
use aver_server::oauth::pkce_s256_challenge;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use tower::ServiceExt;

fn loopback_addr() -> SocketAddr {
    "127.0.0.1:54321".parse().unwrap()
}

fn routable_addr() -> SocketAddr {
    "203.0.113.7:51234".parse().unwrap()
}

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

fn base_config_with_trusted_header(
    dir: &tempfile::TempDir,
    auth_db_path: &std::path::Path,
    header_name: &str,
) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 3317,
        base_url: "http://127.0.0.1:3317".to_string(),
        memory_dir: dir.path().join("memory").to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
        trusted_auth_header: Some(header_name.to_string()),
    }
}

fn register_client(auth_db_path: &std::path::Path, redirect: &str) -> String {
    let db = AuthDb::open(auth_db_path).unwrap();
    let client = db
        .register_client("Test MCP client", &[redirect.to_string()])
        .unwrap();
    client.client_id
}

fn extract_session_cookie(headers: &header::HeaderMap) -> Option<String> {
    let raw = headers.get(header::SET_COOKIE)?.to_str().ok()?;
    raw.split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("aver_session=").map(str::to_string))
}

fn extract_csrf_token(html: &str) -> String {
    let needle = "name=\"csrf_token\" value=\"";
    let i = html.find(needle).expect("csrf_token input present");
    let rest = &html[i + needle.len()..];
    let end = rest.find('"').expect("closing quote");
    rest[..end].to_string()
}

#[tokio::test]
async fn loopback_get_authorize_renders_consent_screen() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);

    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&state=stateA",
        cid = client_id,
        ch = challenge,
    );

    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(loopback_addr()));

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-frame-options")
            .map(|v| v.to_str().unwrap()),
        Some("DENY"),
    );
    assert!(
        extract_session_cookie(response.headers()).is_some(),
        "Set-Cookie should carry aver_session on first GET",
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains(&client_id), "client_id should appear in HTML");
    assert!(
        html.contains("name=\"csrf_token\""),
        "consent screen should contain a CSRF input",
    );
    assert!(
        html.contains("/oauth/authorize/decision"),
        "form should target the decision route",
    );
}

#[tokio::test]
async fn loopback_get_authorize_unknown_client_yields_html_error() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id=unknown&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        ch = challenge,
    );
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(loopback_addr()));

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8",
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Unknown client"));
}

#[tokio::test]
async fn loopback_get_authorize_redirect_uri_mismatch_yields_html_error() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2Fevil.example%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(loopback_addr()));

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Redirect URI mismatch"));
}

/// Drives the full consent flow: GET → POST approve → second GET (skip).
#[tokio::test]
async fn approve_decision_records_consent_redirects_with_code_and_skips_screen() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    // GET to receive cookie + csrf token.
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&state=stateA",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session_cookie =
        extract_session_cookie(response.headers()).expect("cookie set on first GET");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csrf = extract_csrf_token(std::str::from_utf8(&body).unwrap());

    // POST decision=approve.
    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&state=stateA&csrf_token={csrf}&decision=approve",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    assert!(
        response.status() == StatusCode::SEE_OTHER
            || response.status() == StatusCode::TEMPORARY_REDIRECT,
        "expected 3xx, got {}",
        response.status(),
    );
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with("http://127.0.0.1:3917/callback?code="));
    assert!(location.contains("&state=stateA"));

    // Consent row recorded.
    {
        let db = AuthDb::open(&auth_db_path).unwrap();
        let consent = db.get_consent("local", &client_id).unwrap();
        assert!(consent.is_some(), "consent row should exist");
    }

    // Second GET with same scope should skip the screen and 3xx straight to redirect_uri?code=...
    let mut req = Request::builder()
        .uri(&uri)
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.oneshot(req).await.unwrap();
    assert!(
        response.status() == StatusCode::SEE_OTHER
            || response.status() == StatusCode::TEMPORARY_REDIRECT,
        "skip path should 3xx, got {}",
        response.status(),
    );
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with("http://127.0.0.1:3917/callback?code="));
}

#[tokio::test]
async fn deny_decision_redirects_with_access_denied() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&state=denyState",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    let session_cookie = extract_session_cookie(response.headers()).unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csrf = extract_csrf_token(std::str::from_utf8(&body).unwrap());

    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&state=denyState&csrf_token={csrf}&decision=deny",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.oneshot(req).await.unwrap();
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        location.contains("error=access_denied"),
        "deny redirect missing access_denied: {location}",
    );
    assert!(location.contains("state=denyState"));
}

#[tokio::test]
async fn decision_with_bad_csrf_token_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    // GET to obtain a session cookie (needed before the CSRF check fires).
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    let session_cookie = extract_session_cookie(response.headers()).unwrap();

    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&csrf_token=NOT-A-VALID-TOKEN&decision=approve",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn decision_from_non_loopback_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    // Construct a CSRF token off-band so we are testing the loopback gate,
    // not the CSRF gate. The exact value doesn't matter — the request must
    // be rejected before CSRF validation fires.
    let secret_db = AuthDb::open(&auth_db_path).unwrap();
    let secret = secret_db
        .get_or_create_server_secret(CSRF_SECRET_NAME)
        .unwrap();
    let csrf = compute_csrf_token(&secret, "fake-session", &client_id, &challenge);

    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&csrf_token={csrf}&decision=approve",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(routable_addr()));
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "non-loopback POSTs to /oauth/authorize/decision must be rejected",
    );
}

#[tokio::test]
async fn decision_with_cross_site_origin_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config(&dir, &auth_db_path);
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    // Get a real session + valid csrf (so only Origin causes the failure).
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.clone().oneshot(req).await.unwrap();
    let session_cookie = extract_session_cookie(response.headers()).unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csrf = extract_csrf_token(std::str::from_utf8(&body).unwrap());

    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&csrf_token={csrf}&decision=approve",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .header(header::ORIGIN, "http://evil.example")
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(loopback_addr()));
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_loopback_get_authorize_with_trusted_header_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config_with_trusted_header(&dir, &auth_db_path, "x-forwarded-user");
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(routable_addr()));
    req.headers_mut().append(
        header::HeaderName::from_static("x-forwarded-user"),
        "remote-admin".parse().unwrap(),
    );
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(extract_session_cookie(response.headers()).is_some());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("csrf_token"));
}

#[tokio::test]
async fn non_loopback_authorize_decision_requires_matching_trusted_user_session() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let _ = AuthDb::open(&auth_db_path).unwrap();
    let redirect = "http://127.0.0.1:3917/callback";
    let client_id = register_client(&auth_db_path, redirect);
    let config = base_config_with_trusted_header(&dir, &auth_db_path, "x-forwarded-user");
    let app = build_router(config).unwrap();
    let challenge = pkce_s256_challenge("verifier-abc-1234567890");

    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256",
        cid = client_id,
        ch = challenge,
    );
    let mut req = Request::builder().uri(&uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(routable_addr()));
    req.headers_mut().append(
        header::HeaderName::from_static("x-forwarded-user"),
        "remote-admin".parse().unwrap(),
    );
    let response = app.clone().oneshot(req).await.unwrap();
    let session_cookie =
        extract_session_cookie(response.headers()).expect("cookie set on non-loopback header flow");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csrf = extract_csrf_token(std::str::from_utf8(&body).unwrap());

    let form = format!(
        "client_id={cid}&redirect_uri=http%3A%2F%2F127.0.0.1%3A3917%2Fcallback&code_challenge={ch}&code_challenge_method=S256&csrf_token={csrf}&decision=approve",
        cid = client_id,
        ch = challenge,
        csrf = csrf,
    );
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize/decision")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("aver_session={session_cookie}"))
        .body(Body::from(form))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(routable_addr()));
    req.headers_mut().append(
        header::HeaderName::from_static("x-forwarded-user"),
        "remote-admin".parse().unwrap(),
    );
    let response = app.oneshot(req).await.unwrap();
    assert!(response.status().is_redirection() || response.status().is_success());
}
