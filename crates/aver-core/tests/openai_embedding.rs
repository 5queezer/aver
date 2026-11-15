//! OpenAI-compatible embedding client against a local mock server. No live
//! provider call.
#![cfg(feature = "ollama")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use aver_core::vector::{EmbeddingClient, OpenAiEmbeddingClient};

const KEY: &str = "sk-test-SECRETKEYMARKER";

/// Serves one request with `status` and `body`; returns the base URL and a
/// channel yielding the raw request (head and body) that was received.
fn mock_server(status: &'static str, body: String) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let mut request_body = vec![0; content_length];
        reader.read_exact(&mut request_body).unwrap();
        sender
            .send(head + &String::from_utf8(request_body).unwrap())
            .unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        reader.get_mut().write_all(response.as_bytes()).unwrap();
    });
    (url, receiver)
}

fn client(url: &str, dimensions: usize) -> OpenAiEmbeddingClient {
    OpenAiEmbeddingClient::new(url, "text-embedding-3-small", dimensions, KEY)
        .with_timeout(Duration::from_secs(5))
}

fn embedding_body(values: &[f32]) -> String {
    serde_json::json!({
        "object": "list",
        "data": [{"object": "embedding", "embedding": values, "index": 0}],
        "model": "text-embedding-3-small",
        "usage": {"prompt_tokens": 2, "total_tokens": 2},
    })
    .to_string()
}

#[test]
fn request_follows_the_embeddings_api_contract() {
    let (url, requests) = mock_server("200 OK", embedding_body(&[0.25, 0.5, -1.0]));

    let embedding = client(&url, 3).embed("api-gateway uses envoy").unwrap();

    assert_eq!(embedding, vec![0.25, 0.5, -1.0]);
    let request = requests.recv().unwrap();
    let (head, body) = request.split_once("\r\n").unwrap();
    assert_eq!(head, "POST /v1/embeddings HTTP/1.1");
    assert!(
        body.to_ascii_lowercase().contains(&format!(
            "authorization: bearer {}",
            KEY.to_ascii_lowercase()
        )),
        "{body}"
    );
    let json: serde_json::Value = serde_json::from_str(&body[body.find('{').unwrap()..]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "input": "api-gateway uses envoy",
            "model": "text-embedding-3-small",
            "encoding_format": "float",
            "dimensions": 3,
        })
    );
}

/// An authentication error can echo part of the key. Neither the response
/// body nor the key may reach the error text or the client's Debug output.
#[test]
fn provider_errors_never_carry_the_response_body_or_the_key() {
    let leak = "Incorrect API key provided: sk-test-****MARKER. LEAKMARKER";
    let (url, _requests) = mock_server(
        "401 Unauthorized",
        serde_json::json!({"error": {"message": leak}}).to_string(),
    );
    let client = client(&url, 3);

    let error = client.embed("text").unwrap_err();

    let rendered = format!("{error} {error:?} {client:?}");
    assert!(rendered.contains("HTTP 401"), "{rendered}");
    for secret in ["LEAKMARKER", "SECRETKEYMARKER", "Incorrect API key", KEY] {
        assert!(!rendered.contains(secret), "{secret} leaked: {rendered}");
    }
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[test]
fn malformed_and_wrong_sized_responses_are_errors() {
    let bodies = [
        embedding_body(&[0.1, 0.2]),
        serde_json::json!({"object": "list", "data": []}).to_string(),
        "not json".to_string(),
        serde_json::json!({"data": [{"embedding": "AAAA"}]}).to_string(),
    ];
    for body in bodies {
        let (url, _requests) = mock_server("200 OK", body.clone());
        assert!(client(&url, 3).embed("text").is_err(), "{body}");
    }
}

#[test]
fn unreachable_provider_is_an_error_without_the_key() {
    // Bind and drop: the port is closed again.
    let url = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", listener.local_addr().unwrap())
    };
    let error = client(&url, 3).embed("text").unwrap_err().to_string();
    assert!(!error.contains("SECRETKEYMARKER"), "{error}");
}
