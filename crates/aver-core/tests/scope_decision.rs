//! ADR-0024 — decision-model scope suggestion.

use aver_core::Store;
use aver_core::decision::{DecisionError, MockDecisionClient};
use rusqlite::Connection;

const ALLOWED: [&str; 3] = ["global", "proj/aver", "session/abc"];
const FLOOR: f64 = 0.5;

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

#[test]
fn returns_chosen_scope() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::with_indices(vec![1]);
    let got = store
        .suggest_scope("svc", "uses", "lib", "ctx", &ALLOWED, &client, FLOOR)
        .unwrap();
    assert_eq!(got.as_deref(), Some("proj/aver"));
}

#[test]
fn illegal_scope_errors_without_calling_client() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::with_indices(vec![0]);
    let err = store
        .suggest_scope(
            "s",
            "p",
            "o",
            "ctx",
            &["global", "project:aver"],
            &client,
            FLOOR,
        )
        .expect_err("colon is not a legal scope character");
    assert!(format!("{err:?}").contains("InvalidScope"));
    assert!(client.received().is_empty());
}

#[test]
fn unavailable_and_too_few_options_yield_none() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::new(vec![Err(DecisionError::Unavailable)]);
    assert_eq!(
        store
            .suggest_scope("s", "p", "o", "ctx", &ALLOWED, &client, FLOOR)
            .unwrap(),
        None
    );
    let client = MockDecisionClient::with_indices(vec![0]);
    assert_eq!(
        store
            .suggest_scope("s", "p", "o", "ctx", &["global"], &client, FLOOR)
            .unwrap(),
        None
    );
}

#[test]
fn state_and_options_reach_client() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::with_indices(vec![0]);
    let long = "x".repeat(10_000);
    store
        .suggest_scope("svc", "uses", "lib", &long, &ALLOWED, &client, FLOOR)
        .unwrap();
    let received = client.received();
    assert_eq!(received[0].options, ALLOWED);
    for part in ["svc", "uses", "lib"] {
        assert!(received[0].state.contains(part));
    }
    assert!(received[0].state.len() < 2_000);

    let client = MockDecisionClient::with_indices(vec![0]);
    store
        .suggest_scope(
            "svc",
            "uses",
            "lib",
            "src/main.rs",
            &ALLOWED,
            &client,
            FLOOR,
        )
        .unwrap();
    assert!(client.received()[0].state.contains("src/main.rs"));
}

#[test]
fn suggested_scope_composes_with_candidate_staging() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hi", "test")
        .unwrap();
    let client = MockDecisionClient::with_indices(vec![1]);
    let scope = store
        .suggest_scope("subj", "uses", "obj", "hi", &ALLOWED, &client, FLOOR)
        .unwrap()
        .unwrap();
    let id = store
        .propose_candidate_claim_with_scope(event_id, "subj", "uses", "obj", &scope)
        .unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT scope FROM candidate_claims WHERE id = ?",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, scope);
    assert_eq!(stored, "proj/aver");
}

#[test]
fn options_are_described_by_scope_shape() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::with_indices(vec![0]);
    let allowed = [
        "global",
        "proj/aver",
        "proj/aver/branch/feat/x",
        "session/abc",
        "team",
    ];
    store
        .suggest_scope("s", "p", "o", "ctx", &allowed, &client, FLOOR)
        .unwrap();
    assert_eq!(
        client.received()[0].descriptions,
        Some(vec![
            "applies regardless of project or session".to_string(),
            "specific to project aver".to_string(),
            "specific to branch feat/x of project aver".to_string(),
            "only meaningful within session abc".to_string(),
            "team".to_string(),
        ])
    );
}

#[test]
fn floor_gates_suggestion() {
    let (_dir, store) = open_store();
    let client = MockDecisionClient::with_choices(vec![(1, Some(0.4)), (1, Some(0.95)), (0, None)]);
    let suggest = || {
        store
            .suggest_scope("svc", "uses", "lib", "ctx", &ALLOWED, &client, FLOOR)
            .unwrap()
    };
    assert_eq!(suggest(), None, "below floor keeps the caller default");
    assert_eq!(suggest().as_deref(), Some("proj/aver"));
    assert_eq!(
        suggest(),
        None,
        "missing confidence keeps the caller default"
    );
}
