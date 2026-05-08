use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use aver_core::{vector::OllamaEmbeddingClient, Store};
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

#[derive(Debug, Clone)]
pub struct BeamRunConfig {
    pub dataset_path: PathBuf,
    pub ollama_base_url: String,
    pub embedding_model: String,
    pub generation_model: String,
    pub top_k: usize,
    pub limit_conversations: Option<usize>,
    pub limit_questions: Option<usize>,
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct BeamRunReport {
    pub split: String,
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
    format!(
        "You answer a BEAM memory benchmark question using only the retrieved memory context.\n\
Answer the question directly and concisely. Do not respond with generic meta-commentary about the task or context.\n\
If retrieved memories conflict, explicitly say there is contradictory information and ask which statement is correct.\n\
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

pub fn run_beam100k(config: BeamRunConfig) -> Result<BeamRunReport> {
    let dataset = load_dataset(&config.dataset_path)?;
    let root = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("aver-beam100k"));
    std::fs::create_dir_all(&root)?;

    let embedder = OllamaEmbeddingClient::new(&config.ollama_base_url, &config.embedding_model);
    let judge = OllamaGenerateClient::new(&config.ollama_base_url, &config.generation_model);

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
        let conv_dir = root.join(format!("conv-{}", sanitize_path_segment(&conv.id)));
        let _ = std::fs::remove_dir_all(&conv_dir);
        let store = Store::open(&conv_dir)?;
        ingest_conversation(&store, conv, &embedder, &config.embedding_model)?;

        for question in conv
            .questions
            .iter()
            .take(config.limit_questions.unwrap_or(usize::MAX))
        {
            questions += 1;
            let claims = store.recall_hybrid_claims(&question.question, &embedder, config.top_k)?;
            let contexts: Vec<String> = claims.iter().map(|claim| claim.object.clone()).collect();
            let answer = judge.generate(
                &answer_prompt(&question.question, &question.reference_answer, &contexts),
                false,
            )?;
            eprintln!("  Q [{}] {}", question.ability, question.question);
            eprintln!("    A: {}", safe_truncate(&answer.replace('\n', " "), 240));

            for rubric_item in question.rubric_items() {
                let judge_response = judge.generate(
                    &judge_prompt(&question.question, &answer, &rubric_item),
                    true,
                )?;
                let score = parse_judge_score(&judge_response);
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
        split: dataset.split,
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

fn ingest_conversation(
    store: &Store,
    conv: &BeamConversation,
    embedder: &OllamaEmbeddingClient,
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
        let id = store.add_claim(
            &format!("conversation:{}:message:{i}", conv.id),
            &format!("{}_message", message.role),
            &object,
            "beam100k",
        )?;
        let embedding_text = safe_truncate(&object, 4096);
        let embedding = embedder.embed(embedding_text)?;
        store.add_vector_chunk_with_embedding(id, embedding_text, embedding_model, &embedding)?;
        if (i + 1) % 100 == 0 {
            eprintln!("  embedded {}/{} messages", i + 1, conv.user_messages.len());
        }
    }
    Ok(())
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
