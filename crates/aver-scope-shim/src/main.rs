//! ADR-0022 reference shim binary.
//!
//! Binds to `127.0.0.1:0` (ephemeral TCP per council verdict 2026-05-10),
//! prints the bound URL on stdout, and forwards every HTTP request to the
//! upstream aver-server with `X-Aver-Scope` injected. The proxy itself lives
//! in [`aver_scope_shim::proxy`]; this binary is CLI parsing and startup
//! wiring only.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use aver_scope_shim::derive_scope;
use aver_scope_shim::proxy::{DEFAULT_UPSTREAM_TIMEOUT, ShimConfig, router, scope_header_value};

#[derive(Debug, Parser)]
#[command(
    name = "aver-scope-shim",
    about = "Per-workspace HTTP MCP proxy that injects X-Aver-Scope (ADR-0022)"
)]
struct Cli {
    /// Upstream aver-server MCP URL. Request paths are appended to this base;
    /// a duplicated `/mcp` prefix is collapsed so the shim URL works with or
    /// without a trailing `/mcp`.
    #[arg(
        long,
        env = "AVER_UPSTREAM_URL",
        default_value = "http://127.0.0.1:3317/mcp"
    )]
    upstream: String,
    /// Override the auto-derived scope. When set, skips git derivation.
    #[arg(long)]
    scope: Option<String>,
    /// Working directory to derive scope from. Defaults to the current dir.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Bind address. Defaults to `127.0.0.1:0` (ephemeral TCP).
    #[arg(long, default_value = "127.0.0.1:0")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = match cli.cwd {
        Some(p) => p,
        None => std::env::current_dir().context("getting current dir")?,
    };
    let env_default = std::env::var("AVER_DEFAULT_SCOPE").ok();
    let derived = derive_scope(&cwd, cli.scope.as_deref(), env_default.as_deref());

    // Validate the scope as a legal header value once at startup so a
    // misconfigured scope fails fast instead of producing a 502 per request.
    let scope = scope_header_value(&derived.scope)
        .with_context(|| format!("scope {:?} is not a legal HTTP header value", derived.scope))?;

    let app = router(ShimConfig {
        upstream: cli.upstream.clone(),
        scope: Some(scope),
        upstream_timeout: DEFAULT_UPSTREAM_TIMEOUT,
    })?;

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!(
        "aver-scope-shim: scope={} (source={:?}) upstream={}",
        derived.scope, derived.source, cli.upstream
    );
    println!("http://{bound}");

    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
