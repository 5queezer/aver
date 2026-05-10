use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    Form, Json, Router,
    body::Body,
    extract::{ConnectInfo, Query},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rmcp::transport::{
    StreamableHttpServerConfig,
    streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService},
};
use serde::Deserialize;
use tower_http::cors::{Any as CorsAny, CorsLayer};

use crate::{
    auth::{AuthDb, hash_token},
    config::ServerConfig,
    consent::{ConsentDeps, handle_authorize_decision, handle_loopback_get_authorize},
    mcp::AverMcpService,
    oauth::authorization_server_metadata,
    scopes::{SUPPORTED, parse_scope_list_lossy},
};

/// Per-request bag of OAuth scopes granted by the bearer token. Inserted as
/// an Axum extension by [`validate_bearer_token`] and read inside the MCP
/// service via the `http::request::Parts` extension forwarded by rmcp.
#[derive(Debug, Clone, Default)]
pub struct GrantedScopes(pub Vec<crate::scopes::Scope>);

#[derive(Clone)]
struct HttpState {
    config: ServerConfig,
    auth_db: Arc<Mutex<AuthDb>>,
    consent_deps: Arc<ConsentDeps>,
}

pub fn build_router(config: ServerConfig) -> anyhow::Result<Router> {
    let auth_db = Arc::new(Mutex::new(AuthDb::open(&config.auth_db_path)?));
    let consent_deps = Arc::new(ConsentDeps {
        auth_db: auth_db.clone(),
        base_url: config.base_url.clone(),
    });
    let state = HttpState {
        config,
        auth_db,
        consent_deps,
    };
    let protected_api = Router::new().route("/api/health", get(health)).route_layer(
        axum::middleware::from_fn_with_state(state.clone(), validate_bearer_token),
    );

    let memory_dir = state.config.memory_dir.clone();
    let base_url = state.config.base_url.clone();
    let mcp_service: StreamableHttpService<AverMcpService, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                AverMcpService::open(memory_dir.clone(), base_url.clone())
                    .map_err(std::io::Error::other)
            },
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default(),
        );
    let cors = CorsLayer::new()
        .allow_methods(CorsAny)
        .allow_headers(CorsAny);
    let cors = if state.config.cors_origins.is_empty() {
        cors.allow_origin(CorsAny)
    } else {
        let origins = state
            .config
            .cors_origins
            .iter()
            .map(|origin| origin.parse::<HeaderValue>())
            .collect::<Result<Vec<_>, _>>()?;
        cors.allow_origin(origins)
    };
    let protected_mcp = Router::new()
        .nest_service("/mcp", mcp_service)
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            validate_bearer_token,
        ))
        .layer(cors);

    Ok(Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route("/oauth/register", post(oauth_register))
        .route("/oauth/authorize", get(oauth_authorize))
        .route("/oauth/authorize/decision", post(oauth_authorize_decision))
        .route("/oauth/token", post(oauth_token))
        .merge(protected_api)
        .merge(protected_mcp)
        .with_state(state))
}

async fn oauth_metadata(
    axum::extract::State(state): axum::extract::State<HttpState>,
) -> Json<serde_json::Value> {
    Json(authorization_server_metadata(&state.config.base_url))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn validate_bearer_token(
    axum::extract::State(state): axum::extract::State<HttpState>,
    mut request: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(token) = token else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let granted_raw = {
        let Ok(db) = state.auth_db.lock() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        match db.validate_access_token(&hash_token(token)) {
            Ok(Some((_, scopes_raw))) => scopes_raw,
            Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    };
    // Parse leniently: an unknown token in the persisted column is treated
    // as "no implicit access" — valid scopes still apply.
    let granted = parse_scope_list_lossy(&granted_raw);
    request.extensions_mut().insert(GrantedScopes(granted));
    next.run(request).await.into_response()
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    client_name: Option<String>,
    redirect_uris: Vec<String>,
}

async fn oauth_register(
    axum::extract::State(state): axum::extract::State<HttpState>,
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let db = state
        .auth_db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let client = db
        .register_client(
            request.client_name.as_deref().unwrap_or("Aver MCP client"),
            &request.redirect_uris,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "client_id": client.client_id,
            "client_name": client.client_name,
            "redirect_uris": client.redirect_uris,
            "token_endpoint_auth_method": "none",
        })),
    ))
}

#[derive(Debug, Deserialize)]
struct LegacyAuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    approval_token: Option<String>,
}

/// Dispatcher for `GET /oauth/authorize`.
///
/// ADR-0020 slice 2 introduces the loopback consent screen. To avoid breaking
/// the existing approval_token path (slice 6 removes it), we branch on the
/// peer address: loopback requests get the new consent flow, every other
/// caller stays on the legacy gate.
///
/// `ConnectInfo<SocketAddr>` is only present when the server was started via
/// `into_make_service_with_connect_info`. When absent (most unit tests), we
/// treat the request as non-loopback and fall through to the legacy handler,
/// which preserves all pre-existing test behaviour.
async fn oauth_authorize(
    axum::extract::State(state): axum::extract::State<HttpState>,
    request: Request<Body>,
) -> Response {
    let connect_info = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied();
    let is_loopback = connect_info
        .map(|ConnectInfo(addr)| addr.ip().is_loopback())
        .unwrap_or(false);

    if is_loopback {
        // Re-extract the parts we need; the consent module owns its own
        // Query/Form/header parsing so we just pass the request through.
        let (mut parts, body) = request.into_parts();
        let headers = std::mem::take(&mut parts.headers);
        let query_str = parts.uri.query().unwrap_or("");
        let query: crate::consent::AuthorizeQuery = match serde_urlencoded::from_str(query_str) {
            Ok(q) => q,
            Err(_) => {
                // Fall through to the legacy handler so existing 400 codes
                // for malformed queries are unchanged. Re-build the request.
                let request = Request::from_parts(parts, body);
                return legacy_oauth_authorize(state, request).await;
            }
        };
        let connect = connect_info.expect("checked is_loopback above");
        return handle_loopback_get_authorize(
            axum::extract::State(state.consent_deps.clone()),
            connect,
            Query(query),
            headers,
        )
        .await;
    }

    legacy_oauth_authorize(state, request).await
}

/// The pre-ADR-0020 `approval_token`-gated handler. Preserved verbatim so
/// non-loopback callers keep working until slice 6 retires the path.
async fn legacy_oauth_authorize(state: HttpState, request: Request<Body>) -> Response {
    let query_str = request.uri().query().unwrap_or("");
    let query: LegacyAuthorizeQuery = match serde_urlencoded::from_str(query_str) {
        Ok(q) => q,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if query.response_type != "code" || query.code_challenge_method != "S256" {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(expected_approval_token) = state.config.local_authorization_token.as_deref() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if query.approval_token.as_deref() != Some(expected_approval_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let db = match state.auth_db.lock() {
        Ok(g) => g,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let allows = match db.client_allows_redirect_uri(&query.client_id, &query.redirect_uri) {
        Ok(v) => v,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if !allows {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Legacy approval_token path: grant all six canonical scopes so existing
    // setups keep working until slice 6 retires this branch.
    let legacy_scopes: Vec<String> = SUPPORTED.iter().map(|s| s.to_string()).collect();
    let code = match db.store_authorization_code(
        &query.client_id,
        "local",
        &query.code_challenge,
        &query.redirect_uri,
        &legacy_scopes,
    ) {
        Ok(c) => c,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut redirect_url = match url::Url::parse(&query.redirect_uri) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    redirect_url.query_pairs_mut().append_pair("code", &code);
    if let Some(s) = query.state {
        redirect_url.query_pairs_mut().append_pair("state", &s);
    }
    Redirect::to(redirect_url.as_str()).into_response()
}

async fn oauth_authorize_decision(
    axum::extract::State(state): axum::extract::State<HttpState>,
    request: Request<Body>,
) -> Response {
    // Manually extract ConnectInfo so we can return a structured HTML error
    // when the test scaffold did not register peer addresses.
    let connect_info = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied();
    let Some(connect) = connect_info else {
        return (StatusCode::FORBIDDEN, "loopback connect info missing").into_response();
    };

    let (mut parts, body) = request.into_parts();
    let headers = std::mem::take(&mut parts.headers);
    let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let form: crate::consent::DecisionForm = match serde_urlencoded::from_bytes(&bytes) {
        Ok(f) => f,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    handle_authorize_decision(
        axum::extract::State(state.consent_deps.clone()),
        connect,
        headers,
        Form(form),
    )
    .await
}

#[allow(dead_code)]
fn _force_use_headermap(_h: &HeaderMap) {}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    refresh_token: String,
}

async fn oauth_token(
    axum::extract::State(state): axum::extract::State<HttpState>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let db = state
        .auth_db
        .lock()
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let tokens = match request.grant_type.as_str() {
        "authorization_code" => db
            .exchange_authorization_code_for_tokens(
                &request.code,
                &request.client_id,
                &request.code_verifier,
                &request.redirect_uri,
            )
            .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?,
        "refresh_token" => db
            .refresh_access_token(&request.refresh_token)
            .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?,
        _ => return Err(axum::http::StatusCode::BAD_REQUEST),
    };
    Ok(Json(serde_json::json!({
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "token_type": "Bearer",
    })))
}
