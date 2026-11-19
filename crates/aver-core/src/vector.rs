//! Vector retrieval primitives for v0.2.
//!
//! ADR-0013 keeps the Ollama boundary in Rust; this module starts with the
//! deterministic JSON shape expected by Ollama's local embeddings endpoint.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::de::Error as _;

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ollama http: {0}")]
    Http(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("embedding vector must have a non-zero L2 norm")]
    ZeroVector,
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

pub type Embedder = Box<dyn EmbeddingClient + Send + Sync>;

/// Reads the shared CLI/server embedding configuration once. Returns the
/// client (if enabled) and the model label recorded on vector chunks.
///
/// - Default provider (`AVER_EMBEDDING_PROVIDER` unset or `ollama`):
///   `AVER_EMBEDDING_URL` (an Ollama base URL) opts in; `AVER_EMBEDDING_MODEL`
///   overrides the [`VectorIndexConfig`] default model.
/// - `AVER_EMBEDDING_PROVIDER=openai`: an OpenAI-compatible endpoint.
///   `AVER_EMBEDDING_URL` defaults to `https://api.openai.com/v1`,
///   `AVER_EMBEDDING_MODEL` to `text-embedding-3-small`, and the API key is
///   read from the variable named by `AVER_EMBEDDING_API_KEY_ENV` (default
///   `OPENAI_API_KEY`). Vectors are requested at
///   [`crate::VECTOR_INDEX_DIM`].
///
/// A misconfigured provider is reported on stderr (never with the key).
pub fn embedding_from_env() -> (Option<Embedder>, String) {
    embedding_from_vars(|name| std::env::var(name).ok())
}

/// [`embedding_from_env`] over an arbitrary variable source.
pub fn embedding_from_vars(vars: impl Fn(&str) -> Option<String>) -> (Option<Embedder>, String) {
    let var = |name: &str| {
        vars(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let timeout = var("AVER_EMBEDDING_TIMEOUT_SECS")
        .and_then(|value| parse_timeout_secs(&value))
        .unwrap_or(DEFAULT_EMBEDDING_TIMEOUT);
    match var("AVER_EMBEDDING_PROVIDER").as_deref() {
        None | Some("ollama") => {
            let model = VectorIndexConfig::from_optional_config(
                None,
                var("AVER_EMBEDDING_MODEL").as_deref(),
            )
            .unwrap_or_default()
            .embedding_model;
            (
                ollama_embedder(var("AVER_EMBEDDING_URL"), &model, timeout),
                model,
            )
        }
        Some("openai") => {
            let model = var("AVER_EMBEDDING_MODEL")
                .unwrap_or_else(|| DEFAULT_OPENAI_EMBEDDING_MODEL.to_string());
            let key_var =
                var("AVER_EMBEDDING_API_KEY_ENV").unwrap_or_else(|| "OPENAI_API_KEY".to_string());
            let embedder = match var(&key_var) {
                Some(api_key) => openai_embedder(
                    var("AVER_EMBEDDING_URL")
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                    &model,
                    api_key,
                    timeout,
                ),
                None => {
                    // Settings are never echoed: a key pasted into the wrong
                    // variable must not reach stderr and the parent's logs.
                    eprintln!(
                        "aver: AVER_EMBEDDING_PROVIDER=openai but the variable named by AVER_EMBEDDING_API_KEY_ENV (default OPENAI_API_KEY) is not set, using keyword recall"
                    );
                    None
                }
            };
            (embedder, model)
        }
        Some(_) => {
            eprintln!(
                "aver: unknown AVER_EMBEDDING_PROVIDER (expected ollama or openai), using keyword recall"
            );
            (None, VectorIndexConfig::default().embedding_model)
        }
    }
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

pub(crate) fn normalize_embedding(embedding: &[f32]) -> Result<Vec<f32>, EmbeddingError> {
    let norm = embedding
        .iter()
        .map(|component| f64::from(*component) * f64::from(*component))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return Err(EmbeddingError::ZeroVector);
    }
    Ok(embedding
        .iter()
        .map(|component| (f64::from(*component) / norm) as f32)
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OllamaEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
    truncate: bool,
}

impl<'a> OllamaEmbeddingRequest<'a> {
    pub fn new(model: &'a str, input: &'a str) -> Self {
        Self {
            model,
            input,
            truncate: true,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct OllamaEmbeddingResponse {
    pub embeddings: Vec<Vec<f32>>,
}

/// Default for [`OllamaEmbeddingClient::with_timeout`] and
/// [`embedding_timeout_from_env`]: long enough for a cold model load, short
/// enough that a hanging embedding address cannot stall recall for long.
pub const DEFAULT_EMBEDDING_TIMEOUT: Duration = Duration::from_secs(10);

/// `AVER_EMBEDDING_TIMEOUT_SECS` as a duration (positive whole seconds), or
/// [`DEFAULT_EMBEDDING_TIMEOUT`] when unset or unparsable.
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

#[derive(Debug, Clone)]
pub struct OllamaEmbeddingClient {
    base_url: String,
    model: String,
    timeout: Duration,
    agent: ureq::Agent,
}

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
        format!("{}/api/embed", self.base_url)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn request<'a>(&'a self, prompt: &'a str) -> OllamaEmbeddingRequest<'a> {
        OllamaEmbeddingRequest::new(&self.model, prompt)
    }

    pub fn parse_response_body(&self, body: &str) -> Result<Vec<f32>, serde_json::Error> {
        let response: OllamaEmbeddingResponse = serde_json::from_str(body)?;
        response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| serde_json::Error::custom("Ollama returned no embeddings"))
    }

    pub fn embed(&self, prompt: &str) -> Result<Vec<f32>, EmbeddingError> {
        <Self as EmbeddingClient>::embed(self, prompt)
    }
}

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
#[derive(Clone)]
pub struct OpenAiEmbeddingClient {
    base_url: String,
    model: String,
    dimensions: usize,
    api_key: String,
    timeout: Duration,
    agent: ureq::Agent,
}

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

fn openai_embedder(
    url: String,
    model: &str,
    api_key: String,
    timeout: Duration,
) -> Option<Embedder> {
    Some(Box::new(
        OpenAiEmbeddingClient::new(url, model, crate::VECTOR_INDEX_DIM, api_key)
            .with_timeout(timeout),
    ))
}

fn ollama_embedder(url: Option<String>, model: &str, timeout: Duration) -> Option<Embedder> {
    url.map(|url| {
        Box::new(OllamaEmbeddingClient::new(url, model).with_timeout(timeout)) as Embedder
    })
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

    #[test]
    fn embedding_from_vars_uses_supplied_timeout() {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_secs(2));
            }
        });

        let (embedder, _) = embedding_from_vars(|name| match name {
            "AVER_EMBEDDING_URL" => Some(format!("http://{addr}")),
            "AVER_EMBEDDING_TIMEOUT_SECS" => Some("1".to_string()),
            _ => None,
        });
        let err = embedder
            .unwrap()
            .embed("hello")
            .expect_err("no response must be an error");
        assert!(
            err.to_string().contains("timed out after 1.0s"),
            "the supplied timeout must configure the client: {err}"
        );
        server.join().unwrap();
    }

    /// A socket that accepts and never answers must fail within the
    /// configured timeout instead of blocking recall.
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
