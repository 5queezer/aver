use std::sync::{Arc, Mutex};

use axum::{
    Form, Json, Router,
    body::Body,
    extract::Query,
    http::{HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect},
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
    mcp::AverMcpService,
    oauth::authorization_server_metadata,
};

#[derive(Clone)]
struct HttpState {
    config: ServerConfig,
    auth_db: Arc<Mutex<AuthDb>>,
}

pub fn build_router(config: ServerConfig) -> anyhow::Result<Router> {
    let auth_db = Arc::new(Mutex::new(AuthDb::open(&config.auth_db_path)?));
    let state = HttpState { config, auth_db };
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
    request: Request<Body>,
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
    let token_is_valid = {
        let Ok(db) = state.auth_db.lock() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        match db.validate_access_token(&hash_token(token)) {
            Ok(Some(_)) => true,
            Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    };
    if token_is_valid {
        next.run(request).await.into_response()
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
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
struct AuthorizeRequest {
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

async fn oauth_authorize(
    axum::extract::State(state): axum::extract::State<HttpState>,
    Query(request): Query<AuthorizeRequest>,
) -> Result<Redirect, StatusCode> {
    if request.response_type != "code" || request.code_challenge_method != "S256" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let Some(expected_approval_token) = state.config.local_authorization_token.as_deref() else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if request.approval_token.as_deref() != Some(expected_approval_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let db = state
        .auth_db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !db
        .client_allows_redirect_uri(&request.client_id, &request.redirect_uri)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let code = db
        .store_authorization_code(
            &request.client_id,
            "local",
            &request.code_challenge,
            &request.redirect_uri,
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut redirect_url =
        url::Url::parse(&request.redirect_uri).map_err(|_| StatusCode::BAD_REQUEST)?;
    redirect_url.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = request.state {
        redirect_url.query_pairs_mut().append_pair("state", &state);
    }
    Ok(Redirect::to(redirect_url.as_str()))
}

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
