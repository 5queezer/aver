use axum::{
    Form, Json, Router,
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    auth::{AuthDb, hash_token},
    config::ServerConfig,
    oauth::authorization_server_metadata,
};

#[derive(Clone)]
struct HttpState {
    config: ServerConfig,
}

pub fn build_router(config: ServerConfig) -> anyhow::Result<Router> {
    let state = HttpState { config };
    let protected = Router::new().route("/api/health", get(health)).route_layer(
        axum::middleware::from_fn_with_state(state.clone(), validate_bearer_token),
    );

    Ok(Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route("/oauth/token", post(oauth_token))
        .merge(protected)
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
    let Ok(db) = AuthDb::open(&state.config.auth_db_path) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    match db.validate_access_token(&hash_token(token)) {
        Ok(Some(_)) => next.run(request).await.into_response(),
        Ok(None) => StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: String,
    client_id: String,
    code_verifier: String,
}

async fn oauth_token(
    axum::extract::State(state): axum::extract::State<HttpState>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    if request.grant_type != "authorization_code" {
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }
    let db = AuthDb::open(&state.config.auth_db_path)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let access_token = db
        .exchange_authorization_code(&request.code, &request.client_id, &request.code_verifier)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
    Ok(Json(serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
    })))
}
