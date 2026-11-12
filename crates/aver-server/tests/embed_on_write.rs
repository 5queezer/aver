//! `AVER_EMBED_ON_WRITE=1`: claim writes also store an embedded chunk, so
//! ordinary memories are eligible for hybrid recall without a separate
//! `add_vector_chunk` call. Opt-in; best effort; never fails the claim write.
#![cfg(feature = "ollama")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aver_core::decision::MockDecisionClient;
use aver_core::vector::{EmbeddingClient, EmbeddingError};
use aver_server::tools::AverTools;
use serde_json::{Value, json};

/// Counts requests; answers with a fixed vector or fails.
struct CountingEmbedder {
    calls: Arc<AtomicUsize>,
    fail: bool,
}

impl EmbeddingClient for CountingEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(EmbeddingError::Http(
                "embedding provider returned HTTP 503".into(),
            ))
        } else {
            Ok(vec![1.0, 0.0])
        }
    }
}

fn tools(dir: &std::path::Path, embed_on_write: bool, fail: bool) -> (AverTools, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let tools = AverTools::open_with_decision_client(
        dir,
        Some(Box::new(MockDecisionClient::with_indices(vec![0]))),
    )
    .unwrap()
    .with_embedding_client(Some(Box::new(CountingEmbedder {
        calls: calls.clone(),
        fail,
    })))
    .with_embed_on_write(embed_on_write);
    (tools, calls)
}

fn params<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

fn remember(tools: &AverTools, object: &str) -> Value {
    serde_json::to_value(
        tools
            .remember_claim(params(
                json!({"subject": "api-gateway", "predicate": "uses", "object": object}),
            ))
            .unwrap(),
    )
    .unwrap()
}

fn staged_candidate(tools: &AverTools, object: &str) -> i64 {
    let event = tools
        .record_event(params(json!({
            "session_id": "s", "kind": "message", "payload": format!("uses {object}"),
        })))
        .unwrap();
    tools
        .propose_candidate_claim(params(json!({
            "event_id": event.id, "subject": "api-gateway", "predicate": "uses", "object": object,
        })))
        .unwrap()
        .id
}

fn chunk_records(dir: &std::path::Path) -> Vec<Value> {
    // No log file yet means nothing was written at all.
    std::fs::read_to_string(dir.join("log.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["kind"] == "add_vector_chunk")
        .collect()
}

#[test]
fn unset_is_identical_no_field_no_request_no_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, calls) = tools(dir.path(), false, false);

    let claim = remember(&tools, "envoy");
    let triple = serde_json::to_value(
        tools
            .add_triple(params(json!({
                "subject": "api-gateway", "predicate": "uses", "object": "nginx", "source": "t",
            })))
            .unwrap(),
    )
    .unwrap();

    assert!(claim.get("embedded").is_none(), "{claim}");
    assert_eq!(
        triple,
        json!({"triple_id": triple["triple_id"], "status": "appended"})
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(chunk_records(dir.path()).is_empty());
}

/// Every server path that creates a claim embeds it: remember_claim,
/// add_triple, promote_candidate_claim, triage_candidate_claim and the
/// replacement claim of contradict. The chunk is the canonical claim text,
/// written through the logged chunk path, and replay reproduces it.
#[test]
fn opted_in_writes_get_a_vector_chunk_and_hybrid_recall() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, calls) = tools(dir.path(), true, false);

    let remembered = remember(&tools, "envoy");
    assert_eq!(remembered["embedded"], true);
    let triple = tools
        .add_triple(params(json!({
            "subject": "api-gateway", "predicate": "uses", "object": "nginx", "source": "t",
        })))
        .unwrap();
    assert_eq!(triple.embedded, Some(true));
    let promoted = tools
        .promote_candidate_claim(params(
            json!({"candidate_id": staged_candidate(&tools, "haproxy")}),
        ))
        .unwrap();
    assert_eq!(promoted.embedded, Some(true));
    let triaged = tools
        .triage_candidate_claim(params(
            json!({"candidate_id": staged_candidate(&tools, "traefik")}),
        ))
        .unwrap();
    assert_eq!(triaged.triage, "promote");
    assert_eq!(triaged.embedded, Some(true));
    let contradiction = tools
        .contradict(params(json!({
            "triple_id": triple.triple_id, "reason": "replaced",
            "new_claim": {"subject": "api-gateway", "predicate": "uses", "object": "caddy", "source": "t"},
        })))
        .unwrap();
    assert_eq!(contradiction.embedded, Some(true));
    assert_eq!(calls.load(Ordering::SeqCst), 5);

    let chunks = chunk_records(dir.path());
    assert_eq!(chunks.len(), 5);
    assert_eq!(chunks[0]["claim_id"], remembered["id"]);
    assert_eq!(chunks[0]["text"], "api-gateway uses envoy");
    assert_eq!(chunks[0]["embedding"], json!([1.0, 0.0]));

    let recalled = tools
        .recall(params(json!({"query": "api-gateway envoy"})))
        .unwrap();
    assert_eq!(recalled.retrieval, "hybrid");
    assert_eq!(json!(recalled.triples[0].id), remembered["id"]);

    // Replay rebuilds the same chunks.
    let claim_ids: Vec<i64> = chunks
        .iter()
        .map(|c| c["claim_id"].as_i64().unwrap())
        .collect();
    let snapshot = |dir: &std::path::Path| -> String {
        let store = aver_core::Store::open(dir).unwrap();
        claim_ids
            .iter()
            .map(|id| format!("{:?}", store.list_vector_chunks_for_claim(*id).unwrap()))
            .collect()
    };
    drop(tools);
    let live = snapshot(dir.path());
    aver_core::replay(dir.path(), true).unwrap();
    assert_eq!(live, snapshot(dir.path()));
}

#[test]
fn provider_failure_keeps_the_claim_and_reports_keyword_only() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, calls) = tools(dir.path(), true, true);

    let claim = remember(&tools, "envoy");

    assert_eq!(claim["embedded"], false);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let chunks = chunk_records(dir.path());
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].get("embedding").is_none(), "{}", chunks[0]);
    let recalled = tools
        .recall(params(json!({"query": "api-gateway"})))
        .unwrap();
    assert_eq!(recalled.retrieval, "keyword");
    assert_eq!(json!(recalled.triples[0].id), claim["id"]);
}

/// A write that validation or the privacy filter rejects never reaches the
/// provider: the claim write comes first and nothing is embedded without it.
#[test]
fn rejected_writes_send_nothing_to_the_provider() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, calls) = tools(dir.path(), true, false);

    let private = tools.remember_claim(params(json!({
        "subject": "deploy-key", "predicate": "is",
        "object": "-----BEGIN PRIVATE KEY----- abc",
    })));
    let invalid = tools.remember_claim(params(json!({
        "subject": "api-gateway", "predicate": "no_such_predicate_xyz", "object": "envoy",
    })));

    assert!(private.is_err());
    assert!(invalid.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(chunk_records(dir.path()).is_empty());
}

#[test]
fn opt_in_without_a_provider_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(None)
        .with_embed_on_write(true);

    let claim = remember(&tools, "envoy");

    assert!(claim.get("embedded").is_none(), "{claim}");
    assert!(chunk_records(dir.path()).is_empty());
}

/// Mock OpenAI-compatible endpoint: answers every request with a unit vector
/// of the requested dimension and counts requests.
fn mock_provider() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let mut reader = BufReader::new(stream);
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let mut vector = vec![0.0_f32; request["dimensions"].as_u64().unwrap() as usize];
            vector[0] = 1.0;
            counter.fetch_add(1, Ordering::SeqCst);
            let body = json!({"object": "list", "data": [{"object": "embedding", "embedding": vector, "index": 0}]})
                .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = reader.get_mut().write_all(response.as_bytes());
        }
    });
    (url, calls)
}

/// The default path end to end: the real binary over stdio, `remember_claim`
/// only, an OpenAI-compatible provider. The claim gets a 768-dimension
/// vector, recall is hybrid, and the key appears nowhere.
#[test]
fn remember_claim_over_stdio_is_embedded_and_recalled_by_the_hybrid_path() {
    const KEY: &str = "sk-test-EMBEDONWRITEKEYMARKER";
    let (url, calls) = mock_provider();
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aver-server"))
        .arg("--stdio")
        .env("AVER_MEMORY_DIR", dir.path())
        .env("AVER_EMBED_ON_WRITE", "1")
        .env("AVER_EMBEDDING_PROVIDER", "openai")
        .env("AVER_EMBEDDING_URL", url)
        .env("AVER_EMBEDDING_API_KEY_ENV", "AVER_TEST_EMBEDDING_KEY")
        .env("AVER_TEST_EMBEDDING_KEY", KEY)
        .env_remove("AVER_EMBEDDING_MODEL")
        .env_remove("AVER_DEFAULT_SCOPE")
        .env_remove("AVER_RECALL_RECENCY_WEIGHT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut request = |id: i64, method: &str, params: Value| -> Value {
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        loop {
            let mut line = String::new();
            assert_ne!(stdout.read_line(&mut line).unwrap(), 0, "stdout closed");
            let frame: Value = serde_json::from_str(&line).unwrap();
            if frame["id"] == id {
                return frame;
            }
        }
    };
    let payload = |frame: Value| -> Value {
        assert!(frame.get("error").is_none(), "{frame}");
        serde_json::from_str(frame["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    request(
        1,
        "initialize",
        json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    let claim = payload(request(
        2,
        "tools/call",
        json!({"name": "remember_claim", "arguments": {"subject": "api-gateway", "predicate": "uses", "object": "envoy"}}),
    ));
    let recalled = payload(request(
        3,
        "tools/call",
        json!({"name": "recall", "arguments": {"query": "api-gateway"}}),
    ));
    drop(request);
    drop(stdin);
    let output = child.wait_with_output().unwrap();

    assert_eq!(claim["embedded"], true, "{claim}");
    assert_eq!(recalled["retrieval"], "hybrid", "{recalled}");
    assert_eq!(recalled["triples"][0]["id"], claim["id"]);
    // One request for the write, one for the recall query.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let chunks = chunk_records(dir.path());
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0]["embedding"].as_array().unwrap().len(), 768);
    assert_eq!(chunks[0]["embedding_model"], "text-embedding-3-small");
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!log.contains("EMBEDONWRITEKEYMARKER"));
    assert!(!stderr.contains("EMBEDONWRITEKEYMARKER"), "{stderr}");
}
