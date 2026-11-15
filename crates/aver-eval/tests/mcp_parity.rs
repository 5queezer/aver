//! Roadmap Phase 2 exit: a BEAM run through the MCP endpoint scores the same
//! as the direct harness run. The offline test serves `aver-server`
//! in-process with a deterministic mock embedding client, ingests a fixture
//! dataset both ways into separate stores, and compares retrieved claims,
//! answer prompts and scores. The live variant repeats it with Ollama
//! embeddings when `AVER_LIVE_OLLAMA_URL` is set and is a no-op otherwise.

use std::cell::RefCell;
use std::net::SocketAddr;

use aver_core::vector::{EmbeddingClient, EmbeddingError, OllamaEmbeddingClient};
use aver_eval::beam::{
    BeamDataset, BeamJudgeMode, BeamMemory, BeamProvider, BeamRunConfig, BeamRunReport, BeamTarget,
    McpClient, load_dataset_str, retrieval_weights_for_beam_query, run_beam_dataset,
};
use aver_server::auth::{AuthDb, hash_token};
use aver_server::config::ServerConfig;
use aver_server::http::build_router_with_tools;
use aver_server::tools::AverTools;

const TOKEN: &str = "beam-parity-token";
const TOP_K: usize = 3;

const FIXTURE: &str = r#"{
  "split": "parity",
  "num_conversations": 2,
  "total_questions": 4,
  "conversations": [
    {
      "id": "1",
      "category": "work",
      "title": "Deploy notes",
      "user_messages": [
        {"role": "user", "content": "We deploy the billing service with Nomad on Fridays", "time_anchor": "2024-03-01"},
        {"role": "user", "content": "The billing database is Postgres 15", "time_anchor": "2024-03-02"},
        {"role": "user", "content": "Alice owns the payments dashboard", "time_anchor": "2024-03-03"},
        {"role": "user", "content": "We moved the billing deploys to Kubernetes", "time_anchor": "2024-03-10"},
        {"role": "user", "content": "Lunch is usually ramen near the office", "time_anchor": "2024-03-11"}
      ],
      "total_turns": 5,
      "questions": [
        {"ability": "knowledge_update", "question": "How is the billing service deployed now?", "reference_answer": "Kubernetes", "rubric": ["Mentions Kubernetes"]},
        {"ability": "information_extraction", "question": "Which database does billing use?", "reference_answer": "Postgres 15", "rubric": ["Mentions Postgres"]}
      ]
    },
    {
      "id": "2",
      "category": "personal",
      "title": "Hobbies",
      "user_messages": [
        {"role": "user", "content": "I started learning the cello last spring", "time_anchor": "2024-04-01"},
        {"role": "user", "content": "My cello teacher is called Marta", "time_anchor": "2024-04-08"},
        {"role": "user", "content": "I also run on Tuesdays along the river", "time_anchor": "2024-04-09"},
        {"role": "user", "content": "", "time_anchor": "2024-04-10"}
      ],
      "total_turns": 4,
      "questions": [
        {"ability": "information_extraction", "question": "Who teaches me the cello?", "reference_answer": "Marta", "rubric": ["Mentions Marta"]},
        {"ability": "temporal_reasoning", "question": "When do I go running?", "reference_answer": "Tuesdays", "rubric": ["Mentions Tuesdays", "Mentions river"]}
      ]
    }
  ]
}"#;

/// Deterministic bag-of-words embedding: each lowercase token hashes into one
/// of 16 buckets, plus a constant bias so no text embeds to the zero vector.
/// 16 dimensions keep both stores on core's JSON full-scan vector path.
#[derive(Clone)]
struct HashEmbedder;

impl EmbeddingClient for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vector = vec![0.0_f32; 16];
        vector[0] = 1.0;
        for token in text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            let bucket = token
                .to_ascii_lowercase()
                .bytes()
                .fold(2166136261_u32, |hash, byte| {
                    (hash ^ u32::from(byte)).wrapping_mul(16777619)
                });
            vector[1 + (bucket as usize % 15)] += 1.0;
        }
        Ok(vector)
    }
}

/// `aver-server` on a free loopback port over `memory_dir`, with `embedder`
/// injected as its embedding client. Dropping the runtime stops it.
fn start_server(
    dir: &std::path::Path,
    embedder: Box<dyn EmbeddingClient + Send + Sync>,
) -> (tokio::runtime::Runtime, String) {
    let auth_db_path = dir.join("auth.db");
    AuthDb::open(&auth_db_path)
        .unwrap()
        .store_access_token_hash(
            &hash_token(TOKEN),
            "beam-parity",
            &["claims:read".to_string(), "claims:write".to_string()],
        )
        .unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");
    let memory_dir = dir.join("memory");
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port,
        base_url: base_url.clone(),
        memory_dir: memory_dir.to_string_lossy().to_string(),
        auth_db_path: auth_db_path.to_string_lossy().to_string(),
        cors_origins: Vec::new(),
        trusted_auth_header: None,
    };
    let tools = AverTools::open(&memory_dir)
        .unwrap()
        .with_embedding_client(Some(embedder));
    let app = build_router_with_tools(config, tools).unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (runtime, base_url)
}

fn run_config(target: BeamTarget, data_dir: &std::path::Path) -> BeamRunConfig {
    BeamRunConfig {
        dataset_path: "unused".into(),
        provider: BeamProvider::Ollama,
        ollama_base_url: String::new(),
        openai_base_url: String::new(),
        openai_api_key: None,
        embedding_model: "parity-embedder".to_string(),
        generation_model: String::new(),
        top_k: TOP_K,
        retrieval_alpha: None,
        limit_conversations: None,
        limit_questions: None,
        data_dir: Some(data_dir.to_path_buf()),
        judge_mode: BeamJudgeMode::Generative,
        decision_model: String::new(),
        target,
    }
}

/// Text between `start` and the next `end` (or the end of `text`).
fn section<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let rest = text.split_once(start).map_or("", |(_, rest)| rest);
    rest.split_once(end).map_or(rest, |(section, _)| section)
}

/// Runs the scoring loop with a deterministic stand-in for the LLM: the
/// "answer" is the retrieved context block, and the "judge" scores 1.0 when
/// the rubric's last word appears in that answer. Returns the report and
/// every answer prompt in order.
fn score(
    dataset: &BeamDataset,
    config: &BeamRunConfig,
    embedder: &impl EmbeddingClient,
) -> (BeamRunReport, Vec<String>) {
    let answer_prompts = RefCell::new(Vec::new());
    let generate = |prompt: &str, json_mode: bool| -> anyhow::Result<String> {
        if !json_mode {
            answer_prompts.borrow_mut().push(prompt.to_string());
            return Ok(section(prompt, "RETRIEVED MEMORY CONTEXT:\n", "\n\nANSWER:").to_string());
        }
        let rubric = section(prompt, "RUBRIC CRITERION (what to check): ", "\n");
        let keyword = rubric.split_whitespace().last().unwrap_or_default();
        let answer = section(prompt, "RESPONSE TO EVALUATE: ", "\n\n## SCORING");
        let hit = !keyword.is_empty() && answer.contains(keyword);
        Ok(format!(r#"{{"score": {}}}"#, if hit { 1.0 } else { 0.0 }))
    };
    let report = run_beam_dataset(dataset, config, embedder, &generate, None).unwrap();
    (report, answer_prompts.into_inner())
}

fn assert_direct_and_mcp_agree<E>(embedder: E)
where
    E: EmbeddingClient + Clone + Send + Sync + 'static,
{
    let dataset = load_dataset_str(FIXTURE).unwrap();
    let server_dir = tempfile::tempdir().unwrap();
    let direct_dir = tempfile::tempdir().unwrap();
    let (_runtime, base_url) = start_server(server_dir.path(), Box::new(embedder.clone()));

    // Retrieved claims, question by question, in rank order.
    let client = McpClient::connect(&base_url, TOKEN).unwrap();
    for conv in &dataset.conversations {
        let direct = BeamMemory::open_direct(direct_dir.path(), &conv.id).unwrap();
        let mcp = BeamMemory::Mcp {
            client: &client,
            scope: format!("parity/conv-{}", conv.id),
        };
        direct.ingest(conv, &embedder, "parity-embedder").unwrap();
        mcp.ingest(conv, &embedder, "parity-embedder").unwrap();
        for question in &conv.questions {
            let weights = retrieval_weights_for_beam_query(&question.question, None).unwrap();
            let direct_claims = direct
                .recall(&question.question, &embedder, TOP_K, weights)
                .unwrap();
            let mcp_claims = mcp
                .recall(&question.question, &embedder, TOP_K, weights)
                .unwrap();
            assert!(!direct_claims.is_empty(), "{}", question.question);
            assert_eq!(direct_claims, mcp_claims, "{}", question.question);
        }
    }

    // Whole BEAM loop: identical answer prompts and identical scores.
    let (direct_report, direct_prompts) = score(
        &dataset,
        &run_config(BeamTarget::Direct, &direct_dir.path().join("run")),
        &embedder,
    );
    let (mcp_report, mcp_prompts) = score(
        &dataset,
        &run_config(
            BeamTarget::Mcp {
                base_url,
                token: TOKEN.to_string(),
            },
            &direct_dir.path().join("unused"),
        ),
        &embedder,
    );
    assert_eq!(direct_report.questions, 4);
    assert_eq!(direct_report.rubric_checks, 5);
    assert!(
        direct_report.mean_score > 0.0,
        "fixture must score something"
    );
    assert_eq!(direct_prompts, mcp_prompts);
    assert_eq!(
        serde_json::to_value(&direct_report).unwrap(),
        serde_json::to_value(&mcp_report).unwrap()
    );
}

#[test]
fn beam_over_mcp_matches_direct_harness_with_mock_embeddings() {
    assert_direct_and_mcp_agree(HashEmbedder);
}

/// Same comparison with real Ollama embeddings (768-dim models such as
/// `nomic-embed-text` exercise core's `vec0` index path). Runs only when
/// `AVER_LIVE_OLLAMA_URL` is set, for example:
/// `AVER_LIVE_OLLAMA_URL=http://localhost:11434 cargo test -p aver-eval --test mcp_parity`.
#[test]
fn beam_over_mcp_matches_direct_harness_with_live_ollama() {
    let Ok(url) = std::env::var("AVER_LIVE_OLLAMA_URL") else {
        eprintln!("skipping live Ollama parity: AVER_LIVE_OLLAMA_URL is unset");
        return;
    };
    let model =
        std::env::var("AVER_EMBEDDING_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string());
    assert_direct_and_mcp_agree(OllamaEmbeddingClient::new(url, model));
}
