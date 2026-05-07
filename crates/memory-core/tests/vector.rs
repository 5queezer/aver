//! T9 — v0.2 starts with the deterministic Ollama embeddings request shape
//! before any network I/O. ADR-0013 requires Rust HTTP to local Ollama; ADR-0004
//! supplies HybridRAG's vector side.

use memory_core::vector::OllamaEmbeddingRequest;

#[test]
fn ollama_embedding_request_serializes_model_and_prompt() {
    let request = OllamaEmbeddingRequest::new("nomic-embed-text", "auth service depends on stripe");

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["model"], "nomic-embed-text");
    assert_eq!(json["prompt"], "auth service depends on stripe");
}
