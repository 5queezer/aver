//! T2 — adding a claim writes to the JSONL log first (source of truth,
//! ADR-0005) and mirrors into SQLite. Both views are queryable.
//! T3 — get_claim returns full ADR-0003 metadata: provenance,
//! confidence, status, and source_refs.

use memory_core::{ClaimStatus, Provenance, Store};

#[test]
fn add_claim_appends_to_episodic_log_and_mirrors_to_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .expect("add_claim should succeed");

    // SQLite mirror: claim is queryable by id, with the right s/p/o.
    let row = store
        .get_claim(claim_id)
        .expect("get_claim should return the row");
    assert_eq!(row.subject, "auth_service");
    assert_eq!(row.predicate, "depends_on");
    assert_eq!(row.object, "stripe_sdk");

    // JSONL log: the write was appended.
    let log_path = dir.path().join("log.jsonl");
    let log = std::fs::read_to_string(&log_path).expect("log.jsonl should exist");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one log line for one claim");

    let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["kind"], "add_claim");
    assert_eq!(entry["subject"], "auth_service");
    assert_eq!(entry["predicate"], "depends_on");
    assert_eq!(entry["object"], "stripe_sdk");
    assert_eq!(entry["source"], "test_session");
}

#[test]
fn get_claim_returns_provenance_confidence_status_and_source_refs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let id = store
        .add_claim("a", "rel", "b", "test_session")
        .expect("add_claim should succeed");
    let c = store
        .get_claim(id)
        .expect("get_claim should return the row");

    assert_eq!(c.provenance, Provenance::UserAsserted);
    assert!(
        (c.confidence - 0.95).abs() < f64::EPSILON,
        "default confidence for USER_ASSERTED should be 0.95 (ADR-0003)"
    );
    assert_eq!(c.status, ClaimStatus::Active);
    assert_eq!(c.source_refs, vec!["test_session".to_string()]);
}

/// T4 — log-first invariant is *replay-safe*: the JSONL entry carries the
/// same claim_id as the SQLite row, so a fresh DB rebuilt from the log
/// would assign matching ids. Closes the audit-replay gap pi flagged in
/// the T2 review.
#[test]
fn add_claim_log_entry_claim_id_matches_sqlite_row_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("a", "rel", "b", "test_session")
        .expect("add_claim should succeed");

    // SQLite row's id matches the value returned by add_claim.
    let row = store.get_claim(claim_id).unwrap();
    assert_eq!(row.id, claim_id);

    // The JSONL log entry carries the same claim_id.
    let log_path = dir.path().join("log.jsonl");
    let log = std::fs::read_to_string(&log_path).unwrap();
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(entry["claim_id"], serde_json::json!(claim_id));
}

/// T5 — minimal recall: keyword search over claim text returns matching
/// active claims. This is the v0.1 text-only precursor to HybridRAG recall
/// in ADR-0004 / ADR-0008.
#[test]
fn recall_text_returns_claim_by_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let auth_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .expect("add auth claim");
    store
        .add_claim("billing_worker", "emits", "invoice_event", "test_session")
        .expect("add billing claim");

    let matches = store
        .recall_text("stripe")
        .expect("recall_text should search stored claims");

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, auth_id);
    assert_eq!(matches[0].subject, "auth_service");
    assert_eq!(matches[0].object, "stripe_sdk");
}

#[test]
fn claim_text_renders_subject_predicate_object_for_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();

    let claim = store.get_claim(claim_id).unwrap();

    assert_eq!(claim.text(), "auth_service depends_on stripe_sdk");
}

#[test]
fn consolidate_supersedes_duplicate_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store.add_claim("a", "rel", "b", "s1").unwrap();
    let duplicate = store.add_claim("a", "rel", "b", "s2").unwrap();

    let superseded = store.consolidate().unwrap();

    assert_eq!(superseded, 1);
    assert_eq!(store.get_claim(first).unwrap().status, ClaimStatus::Active);
    assert_eq!(
        store.get_claim(duplicate).unwrap().status,
        ClaimStatus::Superseded
    );
}
