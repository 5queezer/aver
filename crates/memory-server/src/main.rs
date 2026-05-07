use memory_server::{config::ServerConfig, http::build_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ServerConfig::from_env()?;
    let app = build_router(config.clone())?;
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
