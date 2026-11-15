//! Streaming HTTP proxy: forwards every request to the upstream
//! aver-server with `X-Aver-Scope` injected.
//!
//! Bodies stream in both directions — no store-and-forward — so long-lived
//! SSE responses (MCP streamable HTTP) reach the client incrementally and
//! shim memory stays bounded. Request bodies are hard-capped at
//! [`MAX_REQUEST_BODY_BYTES`] (413 beyond); response bodies are never
//! capped. The upstream deadline covers connect through response headers;
//! once headers arrive the body streams without a total timeout.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use axum::body::{Body, HttpBody};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use futures_util::StreamExt as _;

/// Request bodies larger than this are rejected with 413. Responses are
/// never capped: SSE streams are long-lived by design.
pub const MAX_REQUEST_BODY_BYTES: u64 = 16 * 1024 * 1024;

/// Default deadline for the upstream to produce response headers.
pub const DEFAULT_UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the shim's HTTP proxy.
#[derive(Clone)]
pub struct ShimConfig {
    /// Upstream base URL, e.g. `http://127.0.0.1:3317/mcp`.
    pub upstream: String,
    /// Scope to inject as `X-Aver-Scope`. `None` injects nothing (any
    /// client-supplied header is still stripped, never trusted).
    pub scope: Option<HeaderValue>,
    /// Deadline for the upstream to produce response headers.
    pub upstream_timeout: Duration,
}

/// Validate a scope string as a legal HTTP header value. Called once at
/// startup so a misconfigured scope fails fast instead of producing a 502
/// on every request.
pub fn scope_header_value(
    scope: &str,
) -> Result<HeaderValue, axum::http::header::InvalidHeaderValue> {
    HeaderValue::from_str(scope)
}

/// Build the shim's axum router.
pub fn router(config: ShimConfig) -> anyhow::Result<axum::Router> {
    let state = AppState {
        upstream: config.upstream,
        scope: config.scope,
        upstream_timeout: config.upstream_timeout,
        // No total `timeout()` here: it would also cap response-body
        // streaming and kill long-lived SSE channels. The deadline is
        // applied to `send()` (headers) in `forward_inner` instead.
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .context("building reqwest client")?,
    };
    Ok(axum::Router::new()
        .route("/{*rest}", any(forward))
        .route("/", any(forward))
        .with_state(state))
}

/// Join the upstream base URL with an incoming path-and-query.
///
/// The default upstream already ends in `/mcp`; a harness that appends
/// `/mcp` to the shim URL must not be forwarded to `/mcp/mcp`, so a
/// duplicated `/mcp` prefix is collapsed.
pub fn join_upstream(base: &str, path_and_query: &str) -> String {
    let base = base.trim_end_matches('/');
    if path_and_query.is_empty() || path_and_query == "/" {
        return base.to_string();
    }
    let path_only = path_and_query.split('?').next().unwrap_or("");
    let suffix =
        if base.ends_with("/mcp") && (path_only == "/mcp" || path_only.starts_with("/mcp/")) {
            let rest = &path_and_query["/mcp".len()..];
            // "/mcp/" collapses onto the base itself, not a trailing-slash variant.
            if rest == "/" { "" } else { rest }
        } else {
            path_and_query
        };
    format!("{base}{suffix}")
}

#[derive(Clone)]
struct AppState {
    upstream: String,
    scope: Option<HeaderValue>,
    upstream_timeout: Duration,
    client: reqwest::Client,
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

    // Reject oversized uploads up front when the length is known.
    if let Some(len) = parts
        .headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        && len > MAX_REQUEST_BODY_BYTES
    {
        return Ok(too_large_response());
    }

    let method = parts.method.clone();
    let path_and_query = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("");
    let target = join_upstream(&state.upstream, path_and_query);

    let mut req_builder = state.client.request(method, &target);

    // Stream the request body with a hard cap. Chunked bodies have no
    // upfront length, so a mid-stream trip flags the cap and the aborted
    // `send()` below is mapped to 413.
    let too_large = Arc::new(AtomicBool::new(false));
    if !body.is_end_stream() {
        let flag = Arc::clone(&too_large);
        let mut used = 0u64;
        let stream = body.into_data_stream().map(move |chunk| {
            chunk
                .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { err.into() })
                .and_then(|bytes: axum::body::Bytes| {
                    used = used.saturating_add(bytes.len() as u64);
                    if used > MAX_REQUEST_BODY_BYTES {
                        flag.store(true, Ordering::Relaxed);
                        Err("request body too large".into())
                    } else {
                        Ok(bytes)
                    }
                })
        });
        req_builder = req_builder.body(reqwest::Body::wrap_stream(stream));
    }

    let mut forward_headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        // Don't forward hop-by-hop, host, or framing headers (reqwest
        // re-computes those), nor a client-supplied X-Aver-Scope (never
        // trusted — the shim is the authority).
        if is_hop_by_hop(name)
            || matches!(name.as_str(), "host" | "content-length" | "x-aver-scope")
        {
            continue;
        }
        forward_headers.insert(name.clone(), value.clone());
    }
    if let Some(scope) = &state.scope {
        forward_headers.insert(HeaderName::from_static("x-aver-scope"), scope.clone());
    }
    req_builder = req_builder.headers(forward_headers);

    // Deadline covers connect + request upload + response headers only;
    // the response body then streams without a total timeout so SSE
    // channels stay open.
    let sent = tokio::time::timeout(state.upstream_timeout, req_builder.send()).await;
    let upstream_resp = match sent {
        Ok(Ok(resp)) => resp,
        Ok(Err(err)) => {
            if too_large.load(Ordering::Relaxed) {
                return Ok(too_large_response());
            }
            return Err(err).context("sending request to upstream");
        }
        Err(_) => {
            return Ok((
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "upstream produced no response headers within {:?}",
                    state.upstream_timeout
                ),
            )
                .into_response());
        }
    };

    let status = upstream_resp.status();
    let resp_headers = upstream_resp.headers().clone();
    let mut response = Response::builder().status(status);
    if let Some(hs) = response.headers_mut() {
        for (name, value) in resp_headers.iter() {
            if is_hop_by_hop(name) {
                continue;
            }
            hs.insert(name.clone(), value.clone());
        }
    }
    // Stream the response body through: SSE channels stay open and chunk
    // timing is preserved.
    response
        .body(Body::from_stream(upstream_resp.bytes_stream()))
        .context("building forwarded response")
}

/// Hop-by-hop headers that must never cross the proxy in either direction
/// (RFC 9110 §7.6.1). `HeaderName::as_str` is lowercase by invariant.
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn too_large_response() -> Response<Body> {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("request body exceeds {MAX_REQUEST_BODY_BYTES} byte limit"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_strips_duplicated_mcp_prefix() {
        let base = "http://127.0.0.1:3317/mcp";
        assert_eq!(join_upstream(base, "/mcp"), base);
        assert_eq!(join_upstream(base, "/mcp/"), base);
        assert_eq!(join_upstream(base, "/mcp/foo"), format!("{base}/foo"));
        assert_eq!(join_upstream(base, "/mcp?x=1"), format!("{base}?x=1"));
    }

    #[test]
    fn join_appends_other_paths_under_upstream() {
        let base = "http://127.0.0.1:3317/mcp";
        assert_eq!(join_upstream(base, "/foo"), format!("{base}/foo"));
        assert_eq!(join_upstream(base, "/"), base);
        assert_eq!(join_upstream(base, ""), base);
    }

    #[test]
    fn join_does_not_strip_mcp_when_base_has_no_mcp_tail() {
        let base = "http://127.0.0.1:3317";
        assert_eq!(join_upstream(base, "/mcp"), format!("{base}/mcp"));
    }

    #[test]
    fn scope_header_value_rejects_control_characters() {
        assert!(scope_header_value("proj/abc").is_ok());
        assert!(scope_header_value("bad\nvalue").is_err());
        assert!(scope_header_value("bad\rvalue").is_err());
    }
}
