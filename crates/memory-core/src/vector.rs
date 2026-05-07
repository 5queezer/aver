//! Vector retrieval primitives for v0.2.
//!
//! ADR-0013 keeps the Ollama boundary in Rust; this module starts with the
//! deterministic JSON shape expected by Ollama's local embeddings endpoint.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct OllamaEmbeddingRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

impl<'a> OllamaEmbeddingRequest<'a> {
    pub fn new(model: &'a str, prompt: &'a str) -> Self {
        Self { model, prompt }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OllamaEmbeddingResponse {
    pub embedding: Vec<f32>,
}
