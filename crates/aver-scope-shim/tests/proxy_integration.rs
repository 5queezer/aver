//! HTTP-path integration tests for the shim proxy.
//!
//! Each test spawns a real upstream axum server and the shim router on
//! ephemeral localhost ports, then drives the shim with a reqwest client.
//! Everything is in-process and offline.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::any;
use futures_util::StreamExt as _;
use tokio::sync::Notify;

use aver_scope_shim::proxy::{MAX_REQUEST_BODY_BYTES, ShimConfig, router};

/// Spawn `app` on an ephemeral localhost port; return its base URL.
async fn spawn(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// Spawn the shim proxying to `upstream` with the given scope.
async fn spawn_shim(upstream: &str, scope: Option<&str>, timeout: Duration) -> String {
    let app = router(ShimConfig {
        upstream: upstream.to_string(),
        scope: scope.map(|s| HeaderValue::from_str(s).unwrap()),
        upstream_timeout: timeout,
    })
    .unwrap();
    spawn(app).await
}

/// Upstream that reports which `X-Aver-Scope` value (if any) it received.
fn echo_upstream() -> axum::Router {
    async fn echo(headers: HeaderMap) -> String {
        headers
            .get("x-aver-scope")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_else(|| "<absent>".to_string())
    }
    axum::Router::new().route("/{*rest}", any(echo))
}

#[tokio::test]
async fn injects_scope_header() {
    let upstream = spawn(echo_upstream()).await;
    let shim = spawn_shim(&upstream, Some("proj/abc123"), Duration::from_secs(5)).await;

    let body = reqwest::get(format!("{shim}/mcp")).await.unwrap();
    assert_eq!(body.status(), StatusCode::OK);
    assert_eq!(body.text().await.unwrap(), "proj/abc123");
}

#[tokio::test]
async fn no_scope_configured_passes_through_without_header() {
    let upstream = spawn(echo_upstream()).await;
    let shim = spawn_shim(&upstream, None, Duration::from_secs(5)).await;

    // Even a client-supplied header must be stripped: the shim never trusts
    // downstream scope claims.
    let resp = reqwest::Client::new()
        .get(format!("{shim}/mcp"))
        .header("x-aver-scope", "proj/forged")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "<absent>");
}

#[tokio::test]
async fn client_scope_header_is_replaced_by_configured_scope() {
    let upstream = spawn(echo_upstream()).await;
    let shim = spawn_shim(&upstream, Some("proj/real"), Duration::from_secs(5)).await;

    let resp = reqwest::Client::new()
        .get(format!("{shim}/mcp"))
        .header("x-aver-scope", "proj/forged")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.text().await.unwrap(), "proj/real");
}

#[tokio::test]
async fn sse_response_streams_incrementally() {
    // Upstream emits chunk 1, parks until the test releases it, then emits
    // chunk 2. If the shim buffered the whole body, the client could never
    // observe chunk 1 while the upstream is still parked.
    let release = Arc::new(Notify::new());
    let release_in_handler = Arc::clone(&release);
    let upstream = spawn(axum::Router::new().route(
        "/mcp",
        any(move || {
            let release = Arc::clone(&release_in_handler);
            async move {
                let stream = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from("data: one\n\n"))
                })
                .chain(futures_util::stream::once(async move {
                    release.notified().await;
                    Ok(axum::body::Bytes::from("data: two\n\n"))
                }));
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(stream),
                )
                    .into_response()
            }
        }),
    ))
    .await;
    let shim = spawn_shim(&upstream, Some("proj/sse"), Duration::from_secs(5)).await;

    let resp = reqwest::get(format!("{shim}/mcp")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );

    let mut stream = resp.bytes_stream();
    // Chunk 1 must arrive while the upstream is still parked on `release`.
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("first chunk never arrived — shim is buffering the stream")
        .unwrap()
        .unwrap();
    assert_eq!(&first[..], b"data: one\n\n");

    release.notify_one();
    let second = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("second chunk never arrived")
        .unwrap()
        .unwrap();
    assert_eq!(&second[..], b"data: two\n\n");
}

#[tokio::test]
async fn request_body_over_cap_is_rejected_413() {
    let upstream = spawn(echo_upstream()).await;
    let shim = spawn_shim(&upstream, Some("proj/cap"), Duration::from_secs(5)).await;

    // Known Content-Length above the cap: rejected up front.
    let body = vec![b'x'; MAX_REQUEST_BODY_BYTES as usize + 1];
    let resp = reqwest::Client::new()
        .post(format!("{shim}/mcp"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// Upstream that drains the request body before answering.
fn draining_upstream() -> axum::Router {
    async fn drain(req: Request) -> StatusCode {
        let mut stream = req.into_body().into_data_stream();
        while stream.next().await.is_some() {}
        StatusCode::OK
    }
    axum::Router::new().route("/{*rest}", any(drain))
}

#[tokio::test]
async fn chunked_request_body_over_cap_is_rejected_413() {
    // The upstream must actually consume the body; otherwise it can answer
    // before the shim's cap trips and the race is unobservable.
    let upstream = spawn(draining_upstream()).await;
    let shim = spawn_shim(&upstream, Some("proj/cap"), Duration::from_secs(5)).await;

    // Unknown length (chunked): the cap trips mid-stream.
    let half = MAX_REQUEST_BODY_BYTES as usize / 2;
    let chunks: Vec<Result<axum::body::Bytes, std::io::Error>> = vec![
        Ok(axum::body::Bytes::from(vec![b'x'; half])),
        Ok(axum::body::Bytes::from(vec![b'x'; half])),
        Ok(axum::body::Bytes::from(vec![b'x'; 1])),
    ];
    let resp = reqwest::Client::new()
        .post(format!("{shim}/mcp"))
        .body(reqwest::Body::wrap_stream(futures_util::stream::iter(
            chunks,
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn request_body_under_cap_passes_through() {
    async fn echo_body(req: Request) -> String {
        let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }
    let upstream = spawn(axum::Router::new().route("/{*rest}", any(echo_body))).await;
    let shim = spawn_shim(&upstream, Some("proj/ok"), Duration::from_secs(5)).await;

    let payload = "x".repeat(1024 * 1024);
    let resp = reqwest::Client::new()
        .post(format!("{shim}/mcp"))
        .body(payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), payload);
}

#[tokio::test]
async fn upstream_hang_yields_504_gateway_timeout() {
    // Upstream never answers within the shim's (short) header deadline.
    let upstream = spawn(axum::Router::new().route(
        "/{*rest}",
        any(|| async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "too late"
        }),
    ))
    .await;
    let shim = spawn_shim(&upstream, Some("proj/slow"), Duration::from_millis(150)).await;

    let resp = reqwest::get(format!("{shim}/mcp")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn duplicated_mcp_path_is_collapsed() {
    // Upstream only serves exactly `/mcp`; anything else is 404. A harness
    // appending `/mcp` to the shim URL must still reach it.
    async fn mcp(req: Request) -> StatusCode {
        if req.uri().path() == "/mcp" {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        }
    }
    let upstream = spawn(
        axum::Router::new()
            .route("/{*rest}", any(mcp))
            .route("/", any(mcp)),
    )
    .await;
    // Shim upstream base already ends in /mcp.
    let shim = spawn_shim(
        &format!("{upstream}/mcp"),
        Some("proj/p"),
        Duration::from_secs(5),
    )
    .await;

    for path in ["/mcp", "/mcp/", ""] {
        let resp = reqwest::get(format!("{shim}{path}")).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "path {path:?} must reach upstream /mcp"
        );
    }
}
