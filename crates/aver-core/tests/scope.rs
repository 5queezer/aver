//! ADR-0021 — scope as a first-class memory dimension.
//!
//! Layer 1 (this file's scope): the `scope` column exists on the four
//! memory-bearing tables, defaults to `'global'`, enforces a charset of
//! `[A-Za-z0-9_/-]`, and refuses blanks. Public types carry `scope` so
//! readers can distinguish a project-local claim from a global one.
//!
//! Read-path semantics (recall walk modes) are tested in a separate file
//! once Task #3 lands.

use aver_core::{ObservationRelevance, Store};
use rusqlite::Connection;

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

#[test]
fn claim_default_scope_is_global() {
    let (dir, store) = open_store();
    let claim_id = store.add_claim("subj_a", "uses", "obj_a", "test").unwrap();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row("SELECT scope FROM claims WHERE id = ?", [claim_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(scope, "global");
}

#[test]
fn episodic_event_default_scope_is_global() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hello", "test")
        .unwrap();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM episodic_events WHERE id = ?",
            [event_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, "global");
}

#[test]
fn observation_default_scope_is_global() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hello", "test")
        .unwrap();
    let obs_id = store
        .record_observation(
            "session_x",
            "test obs",
            ObservationRelevance::Low,
            &[event_id],
            "test",
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
    assert_eq!(scope, "global");
}

#[test]
fn candidate_claim_default_scope_is_global() {
    let (dir, store) = open_store();
    let event_id = store
        .record_event("session_x", "user_message", "hello", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "subj", "uses", "obj")
        .unwrap();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM candidate_claims WHERE id = ?",
            [candidate_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(scope, "global");
}

#[test]
fn claim_explicit_scope_persisted_via_sql() {
    // Until task #2 lands a scope-aware insert API, sanity-check the schema
    // accepts a project-local scope via direct SQL. This proves the column,
    // the index, and the charset trigger are all present and correct.
    let (dir, _store) = open_store();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let now: i64 = 1_700_000_000;
    conn.execute(
        "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                             status, source_refs, agent_id, agent_kind,
                             write_ts, created_at, last_seen_at, scope)
         VALUES (?, 'sub', 'uses', 'obj', 'USER_ASSERTED', 0.9,
                 'ACTIVE', '[\"manual\"]', 'local', 'HUMAN',
                 ?, ?, ?, 'proj/aver')",
        rusqlite::params![1, now, now, now],
    )
    .unwrap();

    let scope: String = conn
        .query_row("SELECT scope FROM claims WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn claim_blank_scope_rejected_by_trigger() {
    let (dir, _store) = open_store();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let now: i64 = 1_700_000_000;
    let err = conn
        .execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind,
                                 write_ts, created_at, last_seen_at, scope)
             VALUES (?, 'sub', 'uses', 'obj', 'USER_ASSERTED', 0.9,
                     'ACTIVE', '[\"manual\"]', 'local', 'HUMAN',
                     ?, ?, ?, '   ')",
            rusqlite::params![2, now, now, now],
        )
        .expect_err("blank scope must be rejected by trigger");
    assert!(
        format!("{err}").contains("scope"),
        "trigger error should mention scope, got: {err}"
    );
}

#[test]
fn claim_invalid_charset_scope_rejected_by_trigger() {
    let (dir, _store) = open_store();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let now: i64 = 1_700_000_000;
    // Space and `@` are outside the allowed charset.
    for bad in ["bad space", "bad@char", "x:y", "x.y", "x?y"] {
        let err = conn
            .execute(
                "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                     status, source_refs, agent_id, agent_kind,
                                     write_ts, created_at, last_seen_at, scope)
                 VALUES (?, 'sub', 'uses', 'obj', 'USER_ASSERTED', 0.9,
                         'ACTIVE', '[\"manual\"]', 'local', 'HUMAN',
                         ?, ?, ?, ?)",
                rusqlite::params![3, now, now, now, bad],
            )
            .expect_err(&format!("expected reject for {bad:?}"));
        assert!(
            format!("{err}").to_lowercase().contains("scope"),
            "trigger error should mention scope for {bad:?}, got: {err}"
        );
    }
}

#[test]
fn claim_valid_path_scopes_accepted_by_trigger() {
    let (dir, _store) = open_store();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let now: i64 = 1_700_000_000;
    let mut id = 100i64;
    for good in [
        "global",
        "proj/aver",
        "proj/aver/branch/feat_x",
        "session/abc-123",
        "proj/with_underscores",
        "proj/MixedCase",
        "proj/12345",
    ] {
        id += 1;
        conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind,
                                 write_ts, created_at, last_seen_at, scope)
             VALUES (?, 'sub', 'uses', 'obj', 'USER_ASSERTED', 0.9,
                     'ACTIVE', '[\"manual\"]', 'local', 'HUMAN',
                     ?, ?, ?, ?)",
            rusqlite::params![id, now, now, now, good],
        )
        .unwrap_or_else(|e| panic!("expected accept for {good:?}, got: {e}"));
    }
}

#[test]
fn claim_struct_carries_scope_field() {
    let (_dir, store) = open_store();
    let _claim_id = store
        .add_claim("scoped_subj", "uses", "scoped_obj", "test")
        .unwrap();
    let claims = store.recall_text("scoped_subj").unwrap();
    let claim = claims
        .iter()
        .find(|c| c.subject == "scoped_subj")
        .expect("recall should surface the just-written claim");
    assert_eq!(claim.scope, "global", "Claim type must expose scope");
}

#[test]
fn episodic_event_struct_carries_scope_field() {
    let (_dir, store) = open_store();
    let event_id = store
        .record_event("session_y", "user_message", "hi", "test")
        .unwrap();
    let event = store.get_event(event_id).unwrap();
    assert_eq!(
        event.scope, "global",
        "EpisodicEvent type must expose scope"
    );
}

#[test]
fn observation_struct_carries_scope_field() {
    let (_dir, store) = open_store();
    let event_id = store
        .record_event("session_y", "user_message", "hi", "test")
        .unwrap();
    let obs_id = store
        .record_observation(
            "session_y",
            "test obs",
            ObservationRelevance::Low,
            &[event_id],
            "test",
        )
        .unwrap();
    let recall = store.recall_observation(&obs_id).unwrap();
    assert_eq!(
        recall.observation.scope, "global",
        "Observation type must expose scope"
    );
}

#[test]
fn candidate_claim_struct_carries_scope_field() {
    let (_dir, store) = open_store();
    let event_id = store
        .record_event("session_y", "user_message", "hi", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "subj", "uses", "obj")
        .unwrap();
    let candidate = store.get_candidate_claim(candidate_id).unwrap();
    assert_eq!(
        candidate.scope, "global",
        "CandidateClaim type must expose scope"
    );
}
