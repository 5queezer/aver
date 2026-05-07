//! Vector retrieval primitives for v0.2.
//!
//! ADR-0013 keeps the Ollama boundary in Rust; this module starts with the
//! deterministic JSON shape expected by Ollama's local embeddings endpoint.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ollama http: {0}")]
    Http(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VectorBackend {
    #[default]
    SqliteVss,
    Qdrant,
}

impl VectorBackend {
    pub fn from_optional_config(value: Option<&str>) -> Result<Self, &'static str> {
        match value {
            Some(value) => value.trim().parse(),
            None => Ok(Self::default()),
        }
    }
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

    pub fn parse_response_body(&self, body: &str) -> Result<Vec<f32>, serde_json::Error> {
        let response: OllamaEmbeddingResponse = serde_json::from_str(body)?;
        Ok(response.embedding)
    }

    pub fn embed(&self, prompt: &str) -> Result<Vec<f32>, EmbeddingError> {
        let response = ureq::post(&self.embeddings_url())
            .send_json(serde_json::to_value(self.request(prompt))?)
            .map_err(|err| EmbeddingError::Http(err.to_string()))?
            .into_string()?;
        Ok(self.parse_response_body(&response)?)
    }
}
