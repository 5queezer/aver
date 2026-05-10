//! ADR-0022 reference shim binary.
//!
//! Binds to `127.0.0.1:0` (ephemeral TCP per council verdict 2026-05-10),
//! prints the bound URL on stdout, and forwards every HTTP request to the
//! upstream aver-server with `X-Aver-Scope` injected.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use clap::Parser;

use aver_scope_shim::derive_scope;

#[derive(Debug, Parser)]
#[command(
    name = "aver-scope-shim",
    about = "Per-workspace HTTP MCP proxy that injects X-Aver-Scope (ADR-0022)"
)]
struct Cli {
    /// Upstream aver-server MCP URL.
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

#[derive(Clone)]
struct AppState {
    upstream: String,
    scope: String,
    client: reqwest::Client,
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

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!(
        "aver-scope-shim: scope={} (source={:?}) upstream={}",
        derived.scope, derived.source, cli.upstream
    );
    println!("http://{bound}");

    let state = AppState {
        upstream: cli.upstream,
        scope: derived.scope,
        client: reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .context("building reqwest client")?,
    };

    let app = axum::Router::new()
        .route("/{*rest}", any(forward))
        .route("/", any(forward))
        .with_state(state);

    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn forward(State(state): State<AppState>, request: Request<Body>) -> Response<Body> {
    match forward_inner(state, request).await {
        Ok(resp) => resp,
        Err(err) => {
            eprintln!("aver-scope-shim: forward error: {err:#}");
            (StatusCode::BAD_GATEWAY, format!("upstream error: {err}")).into_response()
        }
    }
}

async fn forward_inner(state: AppState, request: Request<Body>) -> anyhow::Result<Response<Body>> {
    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let uri: Uri = parts.uri.clone();
    let path_and_query = uri.path_and_query().map(|p| p.as_str()).unwrap_or("");

    // Build target URL: upstream base + path/query of incoming request.
    // ADR-0022: every forwarded request is rewritten to land under upstream.
    let upstream_base: Uri = state.upstream.parse().context("parsing upstream URL")?;
    let upstream_str = if path_and_query.is_empty() || path_and_query == "/" {
        state.upstream.clone()
    } else {
        // If upstream path has a tail like `/mcp`, append the request's tail.
        // Path prefixing: upstream_base.path() + path_and_query — but axum's
        // captured `*rest` is the suffix after `/`. Simpler: concat strings.
        let base = upstream_base.to_string();
        let base = base.trim_end_matches('/');
        format!("{base}{path_and_query}")
    };

    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .context("buffering request body")?;
    let mut req_builder = state
        .client
        .request(method, &upstream_str)
        .body(body_bytes.to_vec());

    let mut forward_headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        // Don't forward hop-by-hop or host headers.
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host"
                | "content-length"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "x-aver-scope"
        ) {
            continue;
        }
        forward_headers.insert(name.clone(), value.clone());
    }
    forward_headers.insert(
        HeaderName::from_static("x-aver-scope"),
        HeaderValue::from_str(&state.scope).context("constructing X-Aver-Scope header value")?,
    );
    req_builder = req_builder.headers(forward_headers);

    let upstream_resp = req_builder
        .send()
        .await
        .context("sending request to upstream")?;
    let status = upstream_resp.status();
    let resp_headers = upstream_resp.headers().clone();
    let resp_bytes = upstream_resp
        .bytes()
        .await
        .context("buffering upstream response body")?;
    let mut response = Response::builder().status(status);
    if let Some(hs) = response.headers_mut() {
        for (name, value) in resp_headers.iter() {
            let lower = name.as_str().to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
            ) {
                continue;
            }
            hs.insert(name.clone(), value.clone());
        }
    }
    response
        .body(Body::from(resp_bytes))
        .context("building forwarded response")
}
