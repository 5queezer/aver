//! Opt-in recall-miss log (roadmap Phase 4 trial): misses and weak hits
//! write one content-free JSON line each; hits and the default (unset env)
//! write nothing.

use std::path::Path;

use aver_server::recall_log::{RECALL_MISS_LOG_FILE, RecallMissLog};
use aver_server::tools::{AddTripleParams, AverTools, RecallParams};

const MISS_QUERY: &str = "zebra quantum lighthouse";

fn recall_params(query: &str) -> RecallParams {
    RecallParams {
        query: query.to_string(),
        hops: None,
        top_k: Some(5),
        scope: None,
        scope_walk: None,
        agent_id: None,
        agent_kind: None,
        predicate: None,
        predicate_walk: None,
        min_confidence: None,
        status: None,
        alpha: None,
    }
}

fn logged_tools(dir: &Path) -> AverTools {
    AverTools::open(dir)
        .unwrap()
        .with_recall_miss_log(Some(RecallMissLog::new(
            dir.join(RECALL_MISS_LOG_FILE),
            0.5,
        )))
}

fn add_claim(tools: &AverTools, subject: &str, object: &str, confidence: f64) {
    tools
        .add_triple(AddTripleParams {
            subject: subject.to_string(),
            predicate: "relates_to".to_string(),
            object: object.to_string(),
            confidence: Some(confidence),
            source: "test".to_string(),
            scope: None,
        })
        .unwrap();
}

fn log_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(RECALL_MISS_LOG_FILE))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn empty_recall_writes_one_content_free_miss_line() {
    let dir = tempfile::tempdir().unwrap();
    let tools = logged_tools(dir.path());
    let view = tools
        .recall_for_agent(recall_params(MISS_QUERY), "aver-test-client")
        .unwrap();
    assert!(view.triples.is_empty());

    let lines = log_lines(dir.path());
    assert_eq!(lines.len(), 1, "{lines:?}");
    let raw = &lines[0];
    assert!(!raw.contains(MISS_QUERY), "query text leaked: {raw}");
    for token in MISS_QUERY.split_whitespace() {
        assert!(!raw.contains(token), "query token {token} leaked: {raw}");
    }
    let event: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(event["kind"], "miss");
    assert_eq!(event["agent_id"], "aver-test-client");
    assert_eq!(event["query_len"], MISS_QUERY.chars().count());
    assert_eq!(event["query_tokens"], 3);
    assert_eq!(event["scope"], "global");
    assert_eq!(event["scope_walk"], "any");
    assert_eq!(event["retrieval"], "keyword");
    assert_eq!(event["alpha"], serde_json::Value::Null);
    assert_eq!(event["top_k"], 5);
    assert_eq!(event["claims_returned"], 0);
    assert_eq!(event["chunks_returned"], 0);
    assert_eq!(event["top_confidence"], serde_json::Value::Null);
    assert_eq!(event["filters"]["status"], "ACTIVE");
    assert_eq!(event["filters"]["agent_id_set"], false);
    assert_eq!(event.as_object().unwrap().len(), 15);
}

#[test]
fn confident_hit_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let tools = logged_tools(dir.path());
    add_claim(&tools, "billing", "ledger", 0.9);
    let view = tools
        .recall_for_agent(recall_params("billing"), "aver-test-client")
        .unwrap();
    assert!(!view.triples.is_empty());
    assert!(
        !dir.path().join(RECALL_MISS_LOG_FILE).exists(),
        "a hit must not create the log"
    );
}

#[test]
fn low_confidence_top_hit_writes_weak_line() {
    let dir = tempfile::tempdir().unwrap();
    let tools = logged_tools(dir.path());
    add_claim(&tools, "billing", "ledger", 0.3);
    tools
        .recall_for_agent(recall_params("billing"), "aver-test-client")
        .unwrap();

    let lines = log_lines(dir.path());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains("billing") && !lines[0].contains("ledger"));
    let event: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(event["kind"], "weak");
    assert_eq!(event["claims_returned"], 1);
    assert_eq!(event["top_confidence"], 0.3);
}

#[test]
fn default_open_without_env_creates_no_log() {
    // AVER_RECALL_MISS_LOG is never set by the test suite.
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    tools
        .recall_for_agent(recall_params(MISS_QUERY), "aver-test-client")
        .unwrap();
    assert!(!dir.path().join(RECALL_MISS_LOG_FILE).exists());
}
