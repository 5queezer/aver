use axum::{Json, Router, routing::get};

use crate::{config::ServerConfig, oauth::authorization_server_metadata};

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
        .with_state(state))
}

async fn oauth_metadata(
    axum::extract::State(state): axum::extract::State<HttpState>,
) -> Json<serde_json::Value> {
    Json(authorization_server_metadata(&state.config.base_url))
}
