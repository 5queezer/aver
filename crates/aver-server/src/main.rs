use std::net::SocketAddr;

use aver_server::{config::ServerConfig, http::build_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ServerConfig::from_env()?;
    let app = build_router(config.clone())?;
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // ConnectInfo<SocketAddr> is required by the ADR-0020 consent flow so
    // GET /oauth/authorize can detect loopback peers.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
