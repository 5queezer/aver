//! ADR-0032 through the server tools: validity params, `reverify_claim`, and
//! recall fields that appear only for claims with a context or confirmation.
//! Invented garden vocabulary.

use aver_server::tools::AverTools;
use serde_json::{Value, json};

fn params<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

fn recall(tools: &AverTools, query: Value) -> Vec<Value> {
    let view = tools.recall(params(query)).unwrap();
    serde_json::to_value(view).unwrap()["triples"]
        .as_array()
        .unwrap()
        .clone()
}

const NEW_FIELDS: [&str; 7] = [
    "context",
    "valid_from",
    "valid_until",
    "last_verified_at",
    "last_confirmed_at",
    "evidence_since_confirmation",
    "latest_confirmation",
];

#[test]
fn recall_shows_validity_only_for_contextual_or_confirmed_claims() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let lesson = tools
        .remember_claim(params(json!({
            "subject": "lesson:shade", "predicate": "concerns", "object": "topic:mulch",
            "scope": "garden", "context": "context:dry",
            "valid_from": 1_700_000_000, "valid_until": 1_700_000_100,
        })))
        .unwrap();
    tools
        .add_triple(params(json!({
            "subject": "bed:north", "predicate": "concerns", "object": "topic:mulch",
            "source": "synthetic", "scope": "garden",
        })))
        .unwrap();

    let triples = recall(&tools, json!({"query": "topic:mulch", "scope": "garden"}));
    let by_subject = |subject: &str| {
        triples
            .iter()
            .find(|t| t["subject"] == subject)
            .unwrap()
            .clone()
    };
    let contextual = by_subject("lesson:shade");
    assert_eq!(contextual["context"], "context:dry");
    assert_eq!(contextual["valid_until"], 1_700_000_100);
    assert_eq!(contextual["evidence_since_confirmation"], 0);
    assert!(contextual["last_confirmed_at"].is_i64());
    assert!(contextual.get("latest_confirmation").is_none());
    let plain = by_subject("bed:north");
    for field in NEW_FIELDS {
        assert!(plain.get(field).is_none(), "{field} on a no-context claim");
    }

    // The context filter.
    let filtered = recall(
        &tools,
        json!({"query": "topic:mulch", "scope": "garden", "context": "context:dry"}),
    );
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0]["id"], lesson.id);

    // Confirm it: the writer and evidence come back on recall.
    let confirmed = tools
        .reverify_claim(params(json!({
            "claim_id": lesson.id, "evidence": ["plot:1"], "scope": "garden",
            "agent_id": "review", "agent_kind": "LLM", "valid_until": 1_700_000_200,
        })))
        .unwrap();
    assert_eq!(confirmed.valid_until, Some(1_700_000_200));
    assert_eq!(confirmed.agent_kind, "LLM");
    let triples = recall(&tools, json!({"query": "topic:mulch", "scope": "garden"}));
    let contextual = triples.iter().find(|t| t["id"] == lesson.id).unwrap();
    assert_eq!(contextual["latest_confirmation"]["agent_id"], "review");
    assert_eq!(
        contextual["latest_confirmation"]["evidence"],
        json!(["plot:1"])
    );
    assert_eq!(contextual["last_confirmed_at"], confirmed.verified_at);
}

#[test]
fn reverify_claim_refuses_another_scope_and_missing_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let lesson = tools
        .remember_claim(params(json!({
            "subject": "lesson:shade", "predicate": "concerns", "object": "topic:mulch",
            "scope": "garden", "context": "context:dry",
        })))
        .unwrap();
    for (call, expected) in [
        (
            json!({"claim_id": lesson.id, "evidence": ["plot:1"], "scope": "replay/r"}),
            "request scope",
        ),
        (
            json!({"claim_id": lesson.id, "evidence": [], "scope": "garden"}),
            "at least one",
        ),
    ] {
        let err = tools.reverify_claim(params(call)).unwrap_err();
        let message = tools.describe_error(&err);
        assert!(message.contains(expected), "{message}");
    }
    let err = tools
        .remember_claim(params(json!({
            "subject": "lesson:sun", "predicate": "concerns", "object": "topic:mulch",
            "valid_from": 1,
        })))
        .unwrap_err();
    assert!(tools.describe_error(&err).contains("need a context"));
}
