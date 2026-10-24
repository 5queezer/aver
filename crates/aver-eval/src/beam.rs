use aver_core::ClaimWrite;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use aver_core::{
    Store,
    decision::{Decision, DecisionClient, DecisionError, OllamaDecisionClient},
    retrieval::HybridWeights,
    vector::{EmbeddingClient, EmbeddingError, OllamaEmbeddingClient},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct BeamDataset {
    pub split: String,
    pub num_conversations: usize,
    pub total_questions: usize,
    pub conversations: Vec<BeamConversation>,
}

#[derive(Debug, Deserialize)]
pub struct BeamConversation {
    pub id: String,
    pub category: String,
    pub title: String,
    pub user_messages: Vec<BeamMessage>,
    pub total_turns: usize,
    pub questions: Vec<BeamQuestion>,
}

#[derive(Debug, Deserialize)]
pub struct BeamMessage {
    pub role: String,
    pub content: String,
    pub time_anchor: String,
}

#[derive(Debug, Deserialize)]
pub struct BeamQuestion {
    pub ability: String,
    pub question: String,
    pub reference_answer: String,
    pub rubric: serde_json::Value,
}

impl BeamQuestion {
    pub fn rubric_items(&self) -> Vec<String> {
        rubric_items(&self.rubric)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamProvider {
    Ollama,
    OpenAi,
}

impl FromStr for BeamProvider {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ollama" => Ok(Self::Ollama),
            "openai" => Ok(Self::OpenAi),
            other => anyhow::bail!("unknown BEAM provider: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BeamJudgeMode {
    #[default]
    Generative,
    Decision,
}

impl FromStr for BeamJudgeMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "generative" => Ok(Self::Generative),
            "decision" => Ok(Self::Decision),
            other => anyhow::bail!("unknown BEAM judge mode: {other}"),
        }
    }
}

impl BeamJudgeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generative => "generative",
            Self::Decision => "decision",
        }
    }
}

/// Where a BEAM run ingests and recalls memory. Answering and judging are the
/// same for every target, so two targets score the same exactly when they
/// retrieve the same contexts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BeamTarget {
    /// One fresh local store per conversation, called in-process.
    #[default]
    Direct,
    /// A running `aver-server`, through its MCP tools (Streamable HTTP with a
    /// bearer token). Each conversation goes into its own scope under one
    /// run-unique prefix and is recalled with `scope_walk = exact`.
    Mcp { base_url: String, token: String },
}

#[derive(Debug, Clone)]
pub struct BeamRunConfig {
    pub dataset_path: PathBuf,
    pub provider: BeamProvider,
    pub ollama_base_url: String,
    pub openai_base_url: String,
    pub openai_api_key: Option<String>,
    pub embedding_model: String,
    pub generation_model: String,
    pub top_k: usize,
    pub retrieval_alpha: Option<f64>,
    pub limit_conversations: Option<usize>,
    pub limit_questions: Option<usize>,
    pub data_dir: Option<PathBuf>,
    pub judge_mode: BeamJudgeMode,
    pub decision_model: String,
    pub target: BeamTarget,
}

#[derive(Debug, Serialize)]
pub struct BeamRunReport {
    pub split: String,
    pub judge_mode: String,
    pub conversations: usize,
    pub questions: usize,
    pub rubric_checks: usize,
    pub mean_score: f64,
    pub ability_scores: HashMap<String, AbilityScore>,
}

#[derive(Debug, Serialize, Default)]
pub struct AbilityScore {
    pub checks: usize,
    pub score_sum: f64,
    pub mean_score: f64,
}

pub fn load_dataset_str(json: &str) -> Result<BeamDataset> {
    serde_json::from_str(json).context("parse BEAM dataset JSON")
}

pub fn load_dataset(path: impl AsRef<Path>) -> Result<BeamDataset> {
    let path = path.as_ref();
    load_dataset_str(
        &std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
    )
}

pub fn resolve_dataset_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return existing_dataset(path);
    }
    if let Ok(path) = std::env::var("BEAM_DATASET_PATH") {
        return existing_dataset(path.into());
    }
    for candidate in [
        PathBuf::from("data/beam-100k.json"),
        PathBuf::from("../karta/data/beam-100k.json"),
        PathBuf::from("../karta/benchmarks/beam-100k.json"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("BEAM dataset not found; pass --dataset or set BEAM_DATASET_PATH")
}

fn existing_dataset(path: PathBuf) -> Result<PathBuf> {
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!("BEAM dataset path does not exist: {}", path.display())
    }
}

pub fn answer_prompt(question: &str, _reference_answer: &str, contexts: &[String]) -> String {
    let order_note = if asks_for_current_value(question) {
        "Context is newest to oldest.\n"
    } else {
        ""
    };
    format!(
        "You answer a BEAM memory benchmark question using only the retrieved memory context.\n\
Answer the question directly and concisely. Do not respond with generic meta-commentary about the task or context.\n\
If retrieved memories conflict, explicitly say there is contradictory information and ask which statement is correct.\n\
For current, latest, updated, or currently questions, prefer the most recent updated value supported by context instead of treating stale values as contradictions.\n\
{order_note}\
If the context is insufficient, say you do not know.\n\n\
QUESTION:\n{question}\n\n\
RETRIEVED MEMORY CONTEXT:\n{}\n\n\
ANSWER:",
        contexts
            .iter()
            .enumerate()
            .map(|(i, context)| format!("[{}] {}", i + 1, context))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub const BEAM_JUDGE_PROMPT: &str = r#"You are an expert evaluator tasked with judging whether the LLM's response demonstrates compliance with the specified RUBRIC CRITERION.

## EVALUATION INPUTS
- QUESTION (what the user asked): <question>
- RUBRIC CRITERION (what to check): <rubric_item>
- RESPONSE TO EVALUATE: <llm_response>

## SCORING SCALE:
- 1.0: complete compliance.
- 0.5: partial compliance.
- 0.0: no compliance.

Return only JSON: {"score": 1.0, "reason": "..."}"#;

pub fn judge_prompt(question: &str, answer: &str, rubric_item: &str) -> String {
    BEAM_JUDGE_PROMPT
        .replace("<question>", question)
        .replace("<rubric_item>", rubric_item)
        .replace("<llm_response>", safe_truncate(answer, 12000))
}

pub const DECISION_JUDGE_QUESTION: &str =
    "How completely does the response to evaluate comply with the rubric criterion?";

pub const DECISION_JUDGE_OPTIONS: [&str; 3] = ["yes", "partial", "no"];

/// Criteria for [`DECISION_JUDGE_OPTIONS`], matching the 1.0 / 0.5 / 0.0
/// scale of [`BEAM_JUDGE_PROMPT`].
pub const DECISION_JUDGE_DESCRIPTIONS: [&str; 3] = [
    "complete compliance: the response fully satisfies the rubric criterion",
    "partial compliance: the response satisfies only part of the rubric criterion",
    "no compliance: the response does not satisfy the rubric criterion",
];

pub fn decision_judge_state(question: &str, answer: &str, rubric_item: &str) -> String {
    format!(
        "QUESTION: {question}\nRUBRIC CRITERION: {rubric_item}\nRESPONSE TO EVALUATE: {}",
        safe_truncate(answer, 12000)
    )
}

/// True for a model answer that maps to no option; every other
/// [`DecisionError`] is an infrastructure or validation failure.
fn is_bad_answer(err: &DecisionError) -> bool {
    matches!(err, DecisionError::InvalidChoice { .. })
}

/// Maps an index into [`DECISION_JUDGE_OPTIONS`] to the same buckets as
/// [`parse_judge_score`].
pub fn score_from_choice(index: usize) -> f64 {
    match index {
        0 => 1.0,
        1 => 0.5,
        _ => 0.0,
    }
}

pub fn parse_judge_score(response: &str) -> f64 {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(response) else {
        return 0.0;
    };
    let score = json["score"].as_f64().unwrap_or(0.0);
    if score >= 0.75 {
        1.0
    } else if score >= 0.25 {
        0.5
    } else {
        0.0
    }
}

pub fn retrieval_weights_for_beam_query(
    query: &str,
    alpha: Option<f64>,
) -> Result<aver_core::retrieval::HybridWeights> {
    if let Some(alpha) = alpha {
        aver_core::retrieval::HybridWeights::try_new(alpha).map_err(Into::into)
    } else {
        Ok(aver_core::retrieval::HybridWeights::for_query(query))
    }
}

pub fn run_beam100k(config: BeamRunConfig) -> Result<BeamRunReport> {
    let dataset = load_dataset(&config.dataset_path)?;
    let embedder = BeamEmbeddingClient::from_config(&config)?;
    let judge = BeamGenerateClient::from_config(&config)?;
    let decision_judge = match config.judge_mode {
        BeamJudgeMode::Generative => None,
        BeamJudgeMode::Decision if config.provider == BeamProvider::Ollama => Some(
            OllamaDecisionClient::new(&config.ollama_base_url, &config.decision_model),
        ),
        BeamJudgeMode::Decision => {
            anyhow::bail!("--judge decision requires --provider ollama")
        }
    };
    run_beam_dataset(
        &dataset,
        &config,
        &embedder,
        &|prompt, json_mode| judge.generate(prompt, json_mode),
        decision_judge
            .as_ref()
            .map(|client| client as &dyn DecisionClient),
    )
}

/// The BEAM loop over an already-loaded dataset: ingest each conversation
/// into `config.target`, recall per question, answer with `generate`, and
/// score each rubric item with `decision_judge` (ADR-0024) or, when `None`,
/// with `generate` in JSON mode. `embedder` is used by the direct target
/// only; an MCP target embeds with the server's configured client.
pub fn run_beam_dataset(
    dataset: &BeamDataset,
    config: &BeamRunConfig,
    embedder: &impl EmbeddingClient,
    generate: &dyn Fn(&str, bool) -> Result<String>,
    decision_judge: Option<&dyn DecisionClient>,
) -> Result<BeamRunReport> {
    let root = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("aver-beam100k"));
    let mcp = match &config.target {
        BeamTarget::Direct => {
            std::fs::create_dir_all(&root)?;
            None
        }
        BeamTarget::Mcp { base_url, token } => Some(McpClient::connect(base_url, token)?),
    };
    let run_scope = format!("beam/run-{}", unique_run_id());

    let mut conversations = 0;
    let mut questions = 0;
    let mut rubric_checks = 0;
    let mut score_sum = 0.0;
    let mut ability_scores: HashMap<String, AbilityScore> = HashMap::new();

    for conv in dataset
        .conversations
        .iter()
        .take(config.limit_conversations.unwrap_or(usize::MAX))
    {
        conversations += 1;
        eprintln!(
            "BEAM conv {} [{}]: {} messages, {} questions",
            conv.id,
            conv.category,
            conv.user_messages.len(),
            conv.questions.len()
        );
        let memory = match &mcp {
            None => BeamMemory::open_direct(&root, &conv.id)?,
            Some(client) => BeamMemory::Mcp {
                client,
                scope: format!("{run_scope}/conv-{}", sanitize_path_segment(&conv.id)),
            },
        };
        memory.ingest(conv, embedder, &config.embedding_model)?;

        for question in conv
            .questions
            .iter()
            .take(config.limit_questions.unwrap_or(usize::MAX))
        {
            questions += 1;
            let claims = memory.recall(
                &question.question,
                embedder,
                config.top_k,
                retrieval_weights_for_beam_query(&question.question, config.retrieval_alpha)?,
            )?;
            let contexts = order_contexts_for_question(&question.question, claims);
            let answer = generate(
                &answer_prompt(&question.question, &question.reference_answer, &contexts),
                false,
            )?;
            eprintln!("  Q [{}] {}", question.ability, question.question);
            eprintln!("    A: {}", safe_truncate(&answer.replace('\n', " "), 240));

            for rubric_item in question.rubric_items() {
                let score = if let Some(decision_judge) = decision_judge {
                    let state = decision_judge_state(&question.question, &answer, &rubric_item);
                    match decision_judge.decide(
                        &Decision::new(&state, DECISION_JUDGE_QUESTION, &DECISION_JUDGE_OPTIONS)
                            .with_descriptions(&DECISION_JUDGE_DESCRIPTIONS),
                    ) {
                        Ok(choice) => score_from_choice(choice.index),
                        Err(err) if is_bad_answer(&err) => {
                            eprintln!(
                                "    decision judge gave an unusable answer, scoring 0.0 rubric={}",
                                safe_truncate(&rubric_item, 160)
                            );
                            0.0
                        }
                        Err(err) => return Err(err.into()),
                    }
                } else {
                    let judge_response = generate(
                        &judge_prompt(&question.question, &answer, &rubric_item),
                        true,
                    )?;
                    parse_judge_score(&judge_response)
                };
                rubric_checks += 1;
                score_sum += score;
                let ability = ability_scores.entry(question.ability.clone()).or_default();
                ability.checks += 1;
                ability.score_sum += score;
                eprintln!(
                    "    score={score} rubric={}",
                    safe_truncate(&rubric_item, 160)
                );
            }
        }
    }

    for ability in ability_scores.values_mut() {
        ability.mean_score = if ability.checks == 0 {
            0.0
        } else {
            ability.score_sum / ability.checks as f64
        };
    }

    Ok(BeamRunReport {
        split: dataset.split.clone(),
        judge_mode: config.judge_mode.as_str().to_string(),
        conversations,
        questions,
        rubric_checks,
        mean_score: if rubric_checks == 0 {
            0.0
        } else {
            score_sum / rubric_checks as f64
        },
        ability_scores,
    })
}

/// Confidence `Store::add_claim` assigns; MCP ingestion passes it explicitly
/// to `add_triple` so both targets write identical claims.
const BEAM_CLAIM_CONFIDENCE: f64 = 0.95;
const BEAM_SOURCE: &str = "beam100k";

/// One conversation's memory on a [`BeamTarget`].
pub enum BeamMemory<'a> {
    Direct(Store),
    Mcp {
        client: &'a McpClient,
        scope: String,
    },
}

impl BeamMemory<'_> {
    /// A fresh store for `conv_id` under `root`, replacing any earlier run's.
    pub fn open_direct(root: &Path, conv_id: &str) -> Result<Self> {
        let conv_dir = root.join(format!("conv-{}", sanitize_path_segment(conv_id)));
        let _ = std::fs::remove_dir_all(&conv_dir);
        Ok(Self::Direct(Store::open(&conv_dir)?))
    }

    /// Writes each non-empty message as a claim plus one vector chunk.
    pub fn ingest(
        &self,
        conv: &BeamConversation,
        embedder: &impl EmbeddingClient,
        embedding_model: &str,
    ) -> Result<()> {
        for (i, message) in conv.user_messages.iter().enumerate() {
            if message.content.trim().is_empty() {
                continue;
            }
            let content = sanitize_memory_text(&message.content);
            let object = if message.time_anchor.trim().is_empty() {
                content
            } else {
                format!("[{}] {}", message.time_anchor, content)
            };
            let subject = format!("conversation:{}:message:{i}", conv.id);
            let predicate = format!("{}_message", message.role);
            let embedding_text = safe_truncate(&object, 4096);
            match self {
                Self::Direct(store) => {
                    let id = store.add_claim(
                        ClaimWrite::new(&subject, &predicate, &object, BEAM_SOURCE)
                            .confidence(BEAM_CLAIM_CONFIDENCE),
                    )?;
                    let embedding = embedder.embed(embedding_text)?;
                    store.add_vector_chunk_with_embedding(
                        id,
                        embedding_text,
                        embedding_model,
                        &embedding,
                    )?;
                }
                Self::Mcp { client, scope } => {
                    let triple = client.call_tool(
                        "add_triple",
                        serde_json::json!({
                            "subject": subject,
                            "predicate": predicate,
                            "object": object,
                            "confidence": BEAM_CLAIM_CONFIDENCE,
                            "source": BEAM_SOURCE,
                            "scope": scope,
                        }),
                    )?;
                    let claim_id = triple["triple_id"]
                        .as_i64()
                        .context("add_triple result has no triple_id")?;
                    client.call_tool(
                        "add_vector_chunk",
                        serde_json::json!({"claim_id": claim_id, "text": embedding_text}),
                    )?;
                }
            }
            if (i + 1) % 100 == 0 {
                eprintln!("  embedded {}/{} messages", i + 1, conv.user_messages.len());
            }
        }
        Ok(())
    }

    /// Hybrid recall (ADR-0004) of `(subject, object)` pairs in rank order.
    /// The MCP target refuses a keyword-path answer: it would not be the
    /// retrieval the direct harness measures.
    pub fn recall(
        &self,
        question: &str,
        embedder: &impl EmbeddingClient,
        top_k: usize,
        weights: HybridWeights,
    ) -> Result<Vec<(String, String)>> {
        match self {
            Self::Direct(store) => Ok(store
                .recall_hybrid_claims_with_alpha(question, embedder, top_k, weights)?
                .into_iter()
                .map(|claim| (claim.subject, claim.object))
                .collect()),
            Self::Mcp { client, scope } => {
                let recalled = client.call_tool(
                    "recall",
                    serde_json::json!({
                        "query": question,
                        "top_k": top_k,
                        "scope": scope,
                        "scope_walk": "exact",
                        "alpha": weights.alpha,
                    }),
                )?;
                if recalled["retrieval"] != "hybrid" {
                    anyhow::bail!(
                        "aver-server answered recall with {} retrieval; set AVER_EMBEDDING_URL on the server so BEAM measures hybrid recall",
                        recalled["retrieval"]
                    );
                }
                recalled["triples"]
                    .as_array()
                    .context("recall result has no triples")?
                    .iter()
                    .map(|triple| {
                        let field = |name: &str| {
                            triple[name]
                                .as_str()
                                .map(str::to_string)
                                .with_context(|| format!("recall triple has no {name}"))
                        };
                        Ok((field("subject")?, field("object")?))
                    })
                    .collect()
            }
        }
    }
}

/// Minimal blocking MCP client for `aver-server`'s Streamable HTTP endpoint:
/// one session, bearer-token auth, `tools/call` only.
pub struct McpClient {
    endpoint: String,
    authorization: String,
    session_id: String,
    next_id: std::cell::Cell<u64>,
}

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

impl McpClient {
    /// Opens a session: `initialize`, then `notifications/initialized`.
    /// `base_url` is the server root (`/mcp` is appended unless present).
    pub fn connect(base_url: &str, token: &str) -> Result<Self> {
        let base_url = base_url.trim_end_matches('/');
        let mut client = Self {
            endpoint: if base_url.ends_with("/mcp") {
                base_url.to_string()
            } else {
                format!("{base_url}/mcp")
            },
            authorization: format!("Bearer {token}"),
            session_id: String::new(),
            next_id: std::cell::Cell::new(1),
        };
        let response = client
            .post(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "aver-beam100k", "version": env!("CARGO_PKG_VERSION")},
                },
            }))
            .context("MCP initialize")?;
        client.session_id = response
            .header("Mcp-Session-Id")
            .context("MCP initialize returned no Mcp-Session-Id")?
            .to_string();
        rpc_result(&response.into_string()?, 0).context("MCP initialize")?;
        client
            .post(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .context("MCP notifications/initialized")?;
        Ok(client)
    }

    /// Calls one tool and returns its JSON result (the tool's text content,
    /// parsed). Tool and JSON-RPC errors become `Err`.
    pub fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let body = self
            .post(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }))
            .with_context(|| format!("MCP tools/call {name}"))?
            .into_string()?;
        let result = rpc_result(&body, id).with_context(|| format!("MCP tools/call {name}"))?;
        let text = result["content"][0]["text"]
            .as_str()
            .with_context(|| format!("MCP tools/call {name}: no text content in {result}"))?;
        if result["isError"] == true {
            anyhow::bail!("MCP tools/call {name}: {text}");
        }
        serde_json::from_str(text).with_context(|| format!("MCP tools/call {name}: parse result"))
    }

    fn post(&self, message: &serde_json::Value) -> Result<ureq::Response> {
        let mut request = ureq::post(&self.endpoint)
            .set("Authorization", &self.authorization)
            .set("Accept", "application/json, text/event-stream");
        if !self.session_id.is_empty() {
            request = request
                .set("Mcp-Session-Id", &self.session_id)
                .set("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);
        }
        match request.send_json(message) {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(code, response)) => anyhow::bail!(
                "HTTP {code} from {}: {}",
                self.endpoint,
                response.into_string().unwrap_or_default()
            ),
            Err(err) => Err(err.into()),
        }
    }
}

/// The `result` of the JSON-RPC response with `id`, from a plain JSON body
/// or from the `data:` lines of an SSE stream.
fn rpc_result(body: &str, id: u64) -> Result<serde_json::Value> {
    let trimmed = body.trim_start();
    let messages: Vec<serde_json::Value> = if trimmed.starts_with('{') {
        vec![serde_json::from_str(trimmed)?]
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|payload| !payload.is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?
    };
    let message = messages
        .into_iter()
        .find(|message| message["id"] == id)
        .with_context(|| format!("no JSON-RPC response with id {id} in {body:?}"))?;
    if let Some(error) = message.get("error") {
        anyhow::bail!("JSON-RPC error: {error}");
    }
    Ok(message["result"].clone())
}

fn unique_run_id() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
enum BeamEmbeddingClient {
    Ollama(OllamaEmbeddingClient),
    OpenAi(OpenAiEmbeddingClient),
}

impl BeamEmbeddingClient {
    fn from_config(config: &BeamRunConfig) -> Result<Self> {
        match config.provider {
            BeamProvider::Ollama => Ok(Self::Ollama(OllamaEmbeddingClient::new(
                &config.ollama_base_url,
                &config.embedding_model,
            ))),
            BeamProvider::OpenAi => Ok(Self::OpenAi(OpenAiEmbeddingClient::new(
                &config.openai_base_url,
                config
                    .openai_api_key
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                    .context("OPENAI_API_KEY is required for --provider openai")?,
                &config.embedding_model,
            ))),
        }
    }
}

impl EmbeddingClient for BeamEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        match self {
            Self::Ollama(client) => client.embed(text),
            Self::OpenAi(client) => client.embed(text),
        }
    }
}

#[derive(Debug, Clone)]
enum BeamGenerateClient {
    Ollama(OllamaGenerateClient),
    OpenAi(OpenAiChatClient),
}

impl BeamGenerateClient {
    fn from_config(config: &BeamRunConfig) -> Result<Self> {
        match config.provider {
            BeamProvider::Ollama => Ok(Self::Ollama(OllamaGenerateClient::new(
                &config.ollama_base_url,
                &config.generation_model,
            ))),
            BeamProvider::OpenAi => Ok(Self::OpenAi(OpenAiChatClient::new(
                &config.openai_base_url,
                config
                    .openai_api_key
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                    .context("OPENAI_API_KEY is required for --provider openai")?,
                &config.generation_model,
            ))),
        }
    }

    fn generate(&self, prompt: &str, json_mode: bool) -> Result<String> {
        match self {
            Self::Ollama(client) => client.generate(prompt, json_mode),
            Self::OpenAi(client) => client.generate(prompt, json_mode),
        }
    }
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'a str>,
    options: OllamaGenerateOptions,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, Clone)]
pub struct OllamaGenerateClient {
    base_url: String,
    model: String,
}

impl OllamaGenerateClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn generate(&self, prompt: &str, json_mode: bool) -> Result<String> {
        let request = OllamaGenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
            format: json_mode.then_some("json"),
            options: OllamaGenerateOptions { temperature: 0.0 },
        };
        let response = ureq::post(&format!("{}/api/generate", self.base_url))
            .send_json(serde_json::to_value(request)?)
            .map_err(|err| anyhow::anyhow!("ollama generate: {err}"))?
            .into_json::<OllamaGenerateResponse>()?;
        Ok(response.response)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
}

impl<'a> OpenAiEmbeddingRequest<'a> {
    pub fn new(model: &'a str, input: &'a str) -> Self {
        Self { model, input }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingClient {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiEmbeddingClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    fn embeddings_url(&self) -> String {
        format!("{}/embeddings", self.base_url)
    }
}

impl EmbeddingClient for OpenAiEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let response = ureq::post(&self.embeddings_url())
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::to_value(OpenAiEmbeddingRequest::new(
                &self.model,
                text,
            ))?)
            .map_err(|err| EmbeddingError::Http(format!("openai embeddings: {err}")))?
            .into_string()?;
        let parsed: OpenAiEmbeddingResponse = serde_json::from_str(&response)?;
        parsed
            .data
            .into_iter()
            .next()
            .map(|item| item.embedding)
            .ok_or_else(|| EmbeddingError::Http("openai embeddings: empty data".to_string()))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiChatMessage<'a>>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAiResponseFormat>,
}

impl<'a> OpenAiChatRequest<'a> {
    pub fn new(model: &'a str, prompt: &'a str, json_mode: bool) -> Self {
        Self {
            model,
            messages: vec![OpenAiChatMessage {
                role: "user",
                content: prompt,
            }],
            temperature: 0.0,
            response_format: json_mode.then_some(OpenAiResponseFormat {
                kind: "json_object",
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

#[derive(Debug, Clone)]
pub struct OpenAiChatClient {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiChatClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub fn generate(&self, prompt: &str, json_mode: bool) -> Result<String> {
        let response = ureq::post(&self.chat_url())
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::to_value(OpenAiChatRequest::new(
                &self.model,
                prompt,
                json_mode,
            ))?)
            .map_err(|err| anyhow::anyhow!("openai chat completions: {err}"))?
            .into_json::<OpenAiChatResponse>()?;
        response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .context("openai chat completions: empty choices")
    }
}

pub fn sanitize_memory_text(content: &str) -> String {
    let mut sanitized = String::with_capacity(content.len());
    let mut token = String::new();
    for ch in content.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            token.push(ch);
        } else {
            push_sanitized_token(&mut sanitized, &token);
            token.clear();
            sanitized.push(ch);
        }
    }
    push_sanitized_token(&mut sanitized, &token);
    sanitized
}

fn push_sanitized_token(output: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    // Pure-hex tokens of known hash lengths (MD5/SHA-1/SHA-256 digests, git
    // SHAs) are identifiers, not secrets — keep them for benchmark fidelity.
    if is_known_hash_hex(token) {
        output.push_str(token);
        return;
    }
    if token.len() >= 20
        && matches!(
            aver_core::privacy_filter(&format!("secret {token}")),
            Err(aver_core::PrivacyRejection::HighEntropy)
                | Err(aver_core::PrivacyRejection::AwsAccessKey)
                | Err(aver_core::PrivacyRejection::GitHubPat)
                | Err(aver_core::PrivacyRejection::Jwt)
                | Err(aver_core::PrivacyRejection::OpenAiKey)
                | Err(aver_core::PrivacyRejection::AnthropicKey)
                | Err(aver_core::PrivacyRejection::StripeLiveKey)
        )
    {
        output.push_str("[REDACTED_SECRET]");
    } else {
        output.push_str(token);
    }
}

fn is_known_hash_hex(token: &str) -> bool {
    matches!(token.len(), 32 | 40 | 64) && token.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn order_contexts_by_beam_message_index(contexts: Vec<(String, String)>) -> Vec<String> {
    order_contexts(contexts, false)
}

pub fn order_contexts_for_question(question: &str, contexts: Vec<(String, String)>) -> Vec<String> {
    order_contexts(contexts, asks_for_current_value(question))
}

fn order_contexts(mut contexts: Vec<(String, String)>, recent_first: bool) -> Vec<String> {
    contexts.sort_by(|(left_subject, _), (right_subject, _)| {
        let ordering = beam_message_index(left_subject)
            .cmp(&beam_message_index(right_subject))
            .then_with(|| left_subject.cmp(right_subject));
        if recent_first {
            ordering.reverse()
        } else {
            ordering
        }
    });
    contexts.into_iter().map(|(_, context)| context).collect()
}

fn asks_for_current_value(question: &str) -> bool {
    let normalized = question.to_ascii_lowercase();
    if normalized.contains("as of") {
        return true;
    }
    // Match whole words so "represent"/"presented" do not trip "present".
    normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| {
            matches!(
                token,
                "current"
                    | "currently"
                    | "latest"
                    | "updated"
                    | "recent"
                    | "now"
                    | "today"
                    | "present"
            )
        })
}

fn beam_message_index(subject: &str) -> Option<usize> {
    subject
        .rsplit_once(":message:")
        .and_then(|(_, index)| index.parse().ok())
}

fn rubric_items(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(items) => {
            items.iter().flat_map(rubric_items).collect::<Vec<String>>()
        }
        serde_json::Value::Object(map) => map.values().flat_map(rubric_items).collect(),
        _ => Vec::new(),
    }
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_mode_parses_known_values_and_rejects_others() {
        assert_eq!(
            "generative".parse::<BeamJudgeMode>().unwrap(),
            BeamJudgeMode::Generative
        );
        assert_eq!(
            "decision".parse::<BeamJudgeMode>().unwrap(),
            BeamJudgeMode::Decision
        );
        assert!("bogus".parse::<BeamJudgeMode>().is_err());
        assert_eq!(BeamJudgeMode::default(), BeamJudgeMode::Generative);
    }

    #[test]
    fn decision_judge_state_contains_inputs_and_truncates_answer() {
        let state = decision_judge_state("What port?", "It is 8080.", "Mentions 8080");
        assert!(state.contains("What port?"));
        assert!(state.contains("It is 8080."));
        assert!(state.contains("Mentions 8080"));

        let long = "x".repeat(13000);
        let state = decision_judge_state("q", &long, "r");
        assert!(state.contains(&"x".repeat(12000)));
        assert!(!state.contains(&"x".repeat(12001)));
    }

    #[test]
    fn score_from_choice_maps_buckets() {
        assert_eq!(score_from_choice(0), 1.0);
        assert_eq!(score_from_choice(1), 0.5);
        assert_eq!(score_from_choice(2), 0.0);
    }

    #[test]
    fn only_invalid_choice_is_a_bad_answer() {
        assert!(is_bad_answer(&DecisionError::InvalidChoice {
            raw: "x".into()
        }));
        assert!(!is_bad_answer(&DecisionError::Http("down".into())));
        assert!(!is_bad_answer(&DecisionError::Unavailable));
        assert!(!is_bad_answer(&DecisionError::TooFewOptions { count: 1 }));
    }

    #[test]
    fn decision_judge_options_has_three_entries() {
        assert_eq!(DECISION_JUDGE_OPTIONS.len(), 3);
    }

    #[test]
    fn decision_judge_descriptions_follow_judge_scale() {
        let decision = Decision::new("s", DECISION_JUDGE_QUESTION, &DECISION_JUDGE_OPTIONS)
            .with_descriptions(&DECISION_JUDGE_DESCRIPTIONS);
        assert!(decision.validate().is_ok());
        for (index, prefix) in ["complete", "partial", "no compliance"].iter().enumerate() {
            assert!(DECISION_JUDGE_DESCRIPTIONS[index].starts_with(prefix));
        }
    }
}
