//! Vector retrieval primitives for v0.2.
//!
//! ADR-0013 keeps the Ollama boundary in Rust; this module starts with the
//! deterministic JSON shape expected by Ollama's local embeddings endpoint.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

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
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => value.parse(),
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

pub trait EmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct MockEmbeddingClient {
    embedding: Vec<f32>,
}

impl MockEmbeddingClient {
    pub fn new(embedding: Vec<f32>) -> Self {
        Self { embedding }
    }
}

impl EmbeddingClient for MockEmbeddingClient {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(self.embedding.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorIndexConfig {
    pub backend: VectorBackend,
    pub embedding_model: String,
}

impl Default for VectorIndexConfig {
    fn default() -> Self {
        Self {
            backend: VectorBackend::default(),
            embedding_model: "nomic-embed-text".to_string(),
        }
    }
}

impl VectorIndexConfig {
    pub fn from_optional_config(
        backend: Option<&str>,
        embedding_model: Option<&str>,
    ) -> Result<Self, &'static str> {
        let default = Self::default();
        Ok(Self {
            backend: VectorBackend::from_optional_config(backend)?,
            embedding_model: embedding_model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or(default.embedding_model),
        })
    }
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (&left, &right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }

    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

pub fn normalized_cosine_score(left: &[f32], right: &[f32]) -> Option<f32> {
    cosine_similarity(left, right).map(|score| (score + 1.0) / 2.0)
}

#[cfg(feature = "ollama")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct OllamaEmbeddingRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[cfg(feature = "ollama")]
impl<'a> OllamaEmbeddingRequest<'a> {
    pub fn new(model: &'a str, prompt: &'a str) -> Self {
        Self { model, prompt }
    }
}

#[cfg(feature = "ollama")]
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct OllamaEmbeddingResponse {
    pub embedding: Vec<f32>,
}

/// Default for [`OllamaEmbeddingClient::with_timeout`] and
/// [`embedding_timeout_from_env`]: long enough for a cold model load, short
/// enough that a hanging embedding address cannot stall recall for long.
pub const DEFAULT_EMBEDDING_TIMEOUT: Duration = Duration::from_secs(10);

/// `AVER_EMBEDDING_TIMEOUT_SECS` as a duration (positive whole seconds), or
/// [`DEFAULT_EMBEDDING_TIMEOUT`] when unset or unparsable. Same shape as the
/// decision client's `AVER_DECISION_TIMEOUT_SECS`.
pub fn embedding_timeout_from_env() -> Duration {
    std::env::var("AVER_EMBEDDING_TIMEOUT_SECS")
        .ok()
        .and_then(|value| parse_timeout_secs(&value))
        .unwrap_or(DEFAULT_EMBEDDING_TIMEOUT)
}

fn parse_timeout_secs(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
}

#[cfg(feature = "ollama")]
#[derive(Debug, Clone)]
pub struct OllamaEmbeddingClient {
    base_url: String,
    model: String,
    timeout: Duration,
    agent: ureq::Agent,
}

#[cfg(feature = "ollama")]
impl OllamaEmbeddingClient {
    /// Client with the default request timeout ([`DEFAULT_EMBEDDING_TIMEOUT`]).
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            timeout: DEFAULT_EMBEDDING_TIMEOUT,
            agent: Self::agent(DEFAULT_EMBEDDING_TIMEOUT),
        }
    }

    /// Replaces the overall per-request timeout (connect + send + read).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.agent = Self::agent(timeout);
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(timeout).build()
    }

    /// `EmbeddingError::Http`, saying so explicitly when `err` is a timeout
    /// (an `io::Error` of kind TimedOut/WouldBlock anywhere in the chain).
    fn http_error(&self, err: &ureq::Error) -> EmbeddingError {
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
        while let Some(current) = source {
            if let Some(io) = current.downcast_ref::<std::io::Error>()
                && matches!(
                    io.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
            {
                return EmbeddingError::Http(format!(
                    "embedding request timed out after {:.1}s: {err}",
                    self.timeout.as_secs_f64()
                ));
            }
            source = current.source();
        }
        EmbeddingError::Http(err.to_string())
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
        <Self as EmbeddingClient>::embed(self, prompt)
    }
}

#[cfg(feature = "ollama")]
impl EmbeddingClient for OllamaEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let response = self
            .agent
            .post(&self.embeddings_url())
            .send_json(serde_json::to_value(self.request(text))?)
            .map_err(|err| self.http_error(&err))?
            .into_string()?;
        Ok(self.parse_response_body(&response)?)
    }
}

/// OpenAI model used when the OpenAI-compatible provider is selected
/// without an explicit model.
pub const DEFAULT_OPENAI_EMBEDDING_MODEL: &str = "text-embedding-3-small";

/// Client for an OpenAI-compatible `POST {base_url}/embeddings` endpoint
/// (request: `input`, `model`, `encoding_format: "float"`, `dimensions`;
/// response: `data[0].embedding`).
///
/// The API key is sent as a bearer token and appears nowhere else: `Debug`
/// redacts it, and errors carry the HTTP status or transport error kind
/// only, never the response body (an authentication error can echo part of
/// the key) or the request.
#[cfg(feature = "ollama")]
#[derive(Clone)]
pub struct OpenAiEmbeddingClient {
    base_url: String,
    model: String,
    dimensions: usize,
    api_key: String,
    timeout: Duration,
    agent: ureq::Agent,
}

#[cfg(feature = "ollama")]
impl fmt::Debug for OpenAiEmbeddingClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiEmbeddingClient")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(feature = "ollama")]
impl OpenAiEmbeddingClient {
    /// `dimensions` is requested from the provider and enforced on the
    /// response, so every stored vector has the same length.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        dimensions: usize,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            dimensions,
            api_key: api_key.into(),
            timeout: DEFAULT_EMBEDDING_TIMEOUT,
            agent: ureq::AgentBuilder::new()
                .timeout(DEFAULT_EMBEDDING_TIMEOUT)
                .build(),
        }
    }

    /// Replaces the overall per-request timeout (connect + send + read).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.agent = ureq::AgentBuilder::new().timeout(timeout).build();
        self
    }

    pub fn embeddings_url(&self) -> String {
        format!("{}/embeddings", self.base_url)
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(feature = "ollama")]
impl EmbeddingClient for OpenAiEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        #[derive(serde::Deserialize)]
        struct Response {
            data: Vec<Item>,
        }
        #[derive(serde::Deserialize)]
        struct Item {
            embedding: Vec<f32>,
        }

        let response = self
            .agent
            .post(&self.embeddings_url())
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::json!({
                "input": text,
                "model": self.model,
                "encoding_format": "float",
                "dimensions": self.dimensions,
            }))
            .map_err(|err| {
                EmbeddingError::Http(match err {
                    ureq::Error::Status(code, _) => {
                        format!("embedding provider returned HTTP {code}")
                    }
                    ureq::Error::Transport(transport) => format!(
                        "embedding request failed within {:.1}s limit: {}",
                        self.timeout.as_secs_f64(),
                        transport.kind()
                    ),
                })
            })?
            .into_string()?;
        // A serde error can quote the offending value; keep provider output
        // out of error text altogether.
        let parsed: Response = serde_json::from_str(&response).map_err(|_| {
            EmbeddingError::Http("embedding provider returned an unreadable response".into())
        })?;
        let embedding = parsed
            .data
            .into_iter()
            .next()
            .map(|item| item.embedding)
            .ok_or_else(|| EmbeddingError::Http("embedding provider returned no data".into()))?;
        if embedding.len() != self.dimensions {
            return Err(EmbeddingError::Http(format!(
                "embedding provider returned {} dimensions, expected {}",
                embedding.len(),
                self.dimensions
            )));
        }
        Ok(embedding)
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    #[test]
    fn timeout_env_parsing_falls_back_to_default() {
        assert_eq!(parse_timeout_secs("3"), Some(Duration::from_secs(3)));
        assert_eq!(parse_timeout_secs(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_timeout_secs("0"), None);
        assert_eq!(parse_timeout_secs("-1"), None);
        assert_eq!(parse_timeout_secs("fast"), None);
        assert_eq!(DEFAULT_EMBEDDING_TIMEOUT, Duration::from_secs(10));
    }

    /// A socket that accepts and never answers must fail within the
    /// configured timeout instead of blocking recall.
    #[cfg(feature = "ollama")]
    #[test]
    fn hanging_embedding_endpoint_times_out() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::time::Instant;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            // Accept, read the request, never respond; keep the connection
            // open until the client gives up.
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_secs(1));
            }
        });

        let client = OllamaEmbeddingClient::new(format!("http://{addr}"), "nomic-embed-text")
            .with_timeout(Duration::from_millis(300));
        assert_eq!(client.timeout(), Duration::from_millis(300));
        let started = Instant::now();
        let err = client
            .embed("hello")
            .expect_err("no response must be an error");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "timeout did not fire in time: {elapsed:?}"
        );
        assert!(
            err.to_string().contains("timed out"),
            "error should say it timed out: {err}"
        );
        drop(client);
        server.join().unwrap();
    }
}
