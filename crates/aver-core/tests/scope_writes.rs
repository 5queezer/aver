//! ADR-0021 — scope-aware write paths.
//!
//! New public Store methods that accept an explicit scope, and the
//! provenance flow that carries scope from candidate claims through to the
//! durable claim on promotion.

use aver_core::{ObservationRelevance, Store};
use rusqlite::Connection;

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

#[test]
fn add_claim_with_scope_persists_explicit_scope() {
    let (dir, store) = open_store();
    let claim_id = store
        .add_claim_with_scope("svc_a", "uses", "lib_x", "test", "proj/aver")
        .unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row("SELECT scope FROM claims WHERE id = ?", [claim_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn add_claim_with_scope_rejects_blank_scope() {
    let (_dir, store) = open_store();
    let err = store
        .add_claim_with_scope("svc_a", "uses", "lib_x", "test", "   ")
        .expect_err("blank scope must reject before SQL");
    assert!(format!("{err:?}").to_lowercase().contains("scope"));
}

#[test]
fn add_claim_with_scope_rejects_invalid_charset() {
    let (_dir, store) = open_store();
    for bad in ["bad space", "x@y", "x:y"] {
        let err = store
            .add_claim_with_scope("svc_a", "uses", "lib_x", "test", bad)
            .expect_err(&format!("invalid scope {bad:?} must reject"));
        assert!(format!("{err:?}").to_lowercase().contains("scope"));
    }
}

#[test]
fn record_event_with_scope_persists_explicit_scope() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event_with_scope("session_x", "user_message", "hi", "test", "proj/aver")
        .unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM episodic_events WHERE id = ?",
            [event_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn record_observation_with_scope_persists_explicit_scope() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event_with_scope("session_x", "user_message", "hi", "test", "proj/aver")
        .unwrap();
    let obs_id = store
        .record_observation_with_scope(
            "session_x",
            "obs body",
            ObservationRelevance::Low,
            &[event_id],
            "test",
            "proj/aver",
        )
        .unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM observations WHERE id = ?",
            [&obs_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn propose_candidate_claim_with_scope_persists_explicit_scope() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hi", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim_with_scope(event_id, "subj", "uses", "obj", "proj/aver")
        .unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM candidate_claims WHERE id = ?",
            [candidate_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn candidate_scope_carries_through_promotion() {
    // Per ADR-0021 §"Write-path semantics": promote_candidate_claim copies
    // the candidate's scope onto the durable claim. There is NO scope
    // parameter on promote — scope is fixed at proposal time.
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hi", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim_with_scope(
            event_id,
            "svc_b",
            "uses",
            "lib_y",
            "proj/aver/branch/feat_x",
        )
        .unwrap();
    let claim_id = store.promote_candidate_claim(candidate_id).unwrap();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row("SELECT scope FROM claims WHERE id = ?", [claim_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(scope, "proj/aver/branch/feat_x");
}

#[test]
fn add_vector_chunk_does_not_take_scope_inherits_from_claim() {
    // Vector chunks attach to a claim; their effective scope is the claim's
    // scope. There is no scope column on `vector_chunks`. This test pins
    // that contract so a future contributor doesn't add a per-chunk scope
    // without updating ADR-0021 first.
    let (dir, _store) = open_store();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(vector_chunks)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        !columns.iter().any(|c| c == "scope"),
        "vector_chunks should not carry a per-chunk scope; got columns {columns:?}"
    );
}
