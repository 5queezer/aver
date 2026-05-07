//! T9/T10/T11 — v0.2 starts with deterministic Ollama embeddings JSON shapes
//! before any network I/O. ADR-0013 requires Rust HTTP to local Ollama; ADR-0004
//! supplies HybridRAG's vector side.

use std::io::{Read, Write};
use std::net::TcpListener;

use memory_core::vector::{
    OllamaEmbeddingClient, OllamaEmbeddingRequest, OllamaEmbeddingResponse, VectorBackend,
    cosine_similarity,
};

#[test]
fn ollama_embedding_request_serializes_model_and_prompt() {
    let request = OllamaEmbeddingRequest::new("nomic-embed-text", "auth service depends on stripe");

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["model"], "nomic-embed-text");
    assert_eq!(json["prompt"], "auth service depends on stripe");
}

#[test]
fn ollama_embedding_response_deserializes_embedding_vector() {
    let response: OllamaEmbeddingResponse =
        serde_json::from_str(r#"{"embedding":[0.125,-0.5,1.0]}"#).unwrap();

    assert_eq!(response.embedding, vec![0.125, -0.5, 1.0]);
}

#[test]
fn ollama_embedding_client_normalizes_embeddings_endpoint_url() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434/", "nomic-embed-text");

    assert_eq!(
        client.embeddings_url(),
        "http://localhost:11434/api/embeddings"
    );
}

#[test]
fn ollama_embedding_client_builds_request_with_configured_model() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434", "nomic-embed-text");

    let json = serde_json::to_value(client.request("remembered claim text")).unwrap();

    assert_eq!(json["model"], "nomic-embed-text");
    assert_eq!(json["prompt"], "remembered claim text");
}

#[test]
fn vector_backend_defaults_to_sqlite_vss() {
    assert_eq!(VectorBackend::default(), VectorBackend::SqliteVss);
}

#[test]
fn vector_backend_parses_qdrant_opt_in() {
    assert_eq!(
        "qdrant".parse::<VectorBackend>().unwrap(),
        VectorBackend::Qdrant
    );
}

#[test]
fn vector_backend_displays_config_values() {
    assert_eq!(VectorBackend::SqliteVss.to_string(), "sqlite-vss");
    assert_eq!(VectorBackend::Qdrant.to_string(), "qdrant");
}

#[test]
fn vector_backend_from_optional_config_defaults_when_unset() {
    assert_eq!(
        VectorBackend::from_optional_config(None).unwrap(),
        VectorBackend::SqliteVss
    );
}

#[test]
fn vector_backend_config_parser_trims_whitespace() {
    assert_eq!(
        VectorBackend::from_optional_config(Some(" qdrant ")).unwrap(),
        VectorBackend::Qdrant
    );
}

#[test]
fn ollama_embedding_client_parses_response_body_to_vector() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434", "nomic-embed-text");

    let embedding = client
        .parse_response_body(r#"{"embedding":[0.25,0.75]}"#)
        .unwrap();

    assert_eq!(embedding, vec![0.25, 0.75]);
}

#[test]
fn cosine_similarity_returns_one_for_identical_vectors() {
    let score = cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap();

    assert!((score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn ollama_embedding_client_posts_to_local_embeddings_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let n = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..n]);
        assert!(request.starts_with("POST /api/embeddings "));

        let body = r#"{"embedding":[0.1,0.2]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let client = OllamaEmbeddingClient::new(format!("http://{addr}"), "nomic-embed-text");
    let embedding = client.embed("hello memory").unwrap();

    handle.join().unwrap();
    assert_eq!(embedding, vec![0.1, 0.2]);
}
