use axum::{
    Form, Json, Router,
    routing::{get, post},
};
use serde::Deserialize;

use crate::{auth::AuthDb, config::ServerConfig, oauth::authorization_server_metadata};

#[derive(Clone)]
struct HttpState {
    config: ServerConfig,
}

pub fn build_router(config: ServerConfig) -> anyhow::Result<Router> {
    let state = HttpState { config };
    Ok(Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route("/oauth/token", post(oauth_token))
        .with_state(state))
}

async fn oauth_metadata(
    axum::extract::State(state): axum::extract::State<HttpState>,
) -> Json<serde_json::Value> {
    Json(authorization_server_metadata(&state.config.base_url))
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
