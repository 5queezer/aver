use std::net::SocketAddr;

use aver_server::{
    config::ServerConfig, http::build_router, mcp::AverMcpService, scope_resolution::resolve_scope,
};
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ServerConfig::from_env()?;
    if std::env::args().skip(1).any(|arg| arg == "--stdio") {
        return serve_stdio(config).await;
    }
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

/// `--stdio`: serve MCP to the parent process over stdin/stdout instead of
/// HTTP. No listener is bound and no OAuth runs; the parent is the only
/// client and holds every tool scope. stdout carries protocol frames only,
/// diagnostics go to stderr. Ends when the parent closes stdin.
async fn serve_stdio(config: ServerConfig) -> anyhow::Result<()> {
    let env_default = std::env::var("AVER_DEFAULT_SCOPE").ok();
    let scope = resolve_scope(None, None, None, env_default.as_deref())?;
    let service = AverMcpService::open(&config.memory_dir, config.base_url)?.local_stdio(scope);
    service
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
