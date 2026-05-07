//! T9/T10/T11 — v0.2 starts with deterministic Ollama embeddings JSON shapes
//! before any network I/O. ADR-0013 requires Rust HTTP to local Ollama; ADR-0004
//! supplies HybridRAG's vector side.

use memory_core::vector::{OllamaEmbeddingClient, OllamaEmbeddingRequest, OllamaEmbeddingResponse};

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
