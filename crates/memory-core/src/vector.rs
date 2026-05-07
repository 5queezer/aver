//! Vector retrieval primitives for v0.2.
//!
//! ADR-0013 keeps the Ollama boundary in Rust; this module starts with the
//! deterministic JSON shape expected by Ollama's local embeddings endpoint.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VectorBackend {
    #[default]
    SqliteVss,
    Qdrant,
}

impl FromStr for VectorBackend {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sqlite-vss" => Ok(Self::SqliteVss),
            "qdrant" => Ok(Self::Qdrant),
            _ => Err("unknown vector backend"),
        }
    }
}

impl fmt::Display for VectorBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SqliteVss => "sqlite-vss",
            Self::Qdrant => "qdrant",
        })
    }
}

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

#[derive(Debug, Clone)]
pub struct OllamaEmbeddingClient {
    base_url: String,
    model: String,
}

impl OllamaEmbeddingClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn embeddings_url(&self) -> String {
        format!("{}/api/embeddings", self.base_url)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn request<'a>(&'a self, prompt: &'a str) -> OllamaEmbeddingRequest<'a> {
        OllamaEmbeddingRequest::new(&self.model, prompt)
    }
}
