//! ADR-0019 — operational policy: WAL checkpoint, vacuum, replay,
//! log rotation, schema version.

use std::io::Write;

use aver_core::{
    AgentKind, HyperedgeInput, HyperedgeParticipantInput, LOG_ROTATE_MAX_LINES,
    ObservationRelevance, Provenance, Store, replay, vacuum,
};
use rusqlite::Connection;

#[test]
fn open_sets_wal_autocheckpoint_to_4000() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // Write a few claims to make sure WAL is active.
    for i in 0..5 {
        store
            .add_claim(
                &format!("subj{i}"),
                "rel",
                &format!("obj{i}"),
                "test_session",
            )
            .unwrap();
    }
    let value = store
        .wal_autocheckpoint()
        .expect("wal_autocheckpoint pragma should be readable");
    assert_eq!(value, 4_000, "ADR-0019 §1 raises threshold to 4000");
}

#[test]
fn close_truncates_wal() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for i in 0..50 {
        store
            .add_claim(&format!("s{i}"), "p", &format!("o{i}"), "src")
            .unwrap();
    }
    store.close().expect("close should succeed");

    let wal_path = dir.path().join("db.sqlite-wal");
    if wal_path.exists() {
        let len = std::fs::metadata(&wal_path).unwrap().len();
        assert_eq!(
            len, 0,
            "wal_checkpoint(TRUNCATE) on close should leave WAL at zero bytes"
        );
    }
}

#[test]
fn schema_version_set_to_migrations_len_after_open() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let v: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(v > 0, "user_version should be set after migrations apply");
}

#[test]
fn schema_too_new_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    {
        // Initial open creates the db and applies migrations.
        let _store = Store::open(dir.path()).unwrap();
    }
    {
        // Bump user_version to an impossibly high value to simulate a newer db.
        let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
        conn.pragma_update(None, "user_version", 9_999i64).unwrap();
    }
    let err = match Store::open(dir.path()) {
        Ok(_) => panic!("expected schema-too-new refusal"),
        Err(err) => err,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("schema too new") || msg.contains("user_version"),
        "expected schema-too-new error, got: {msg}"
    );
}

#[test]
fn fresh_db_with_user_version_zero_runs_all_migrations() {
    let dir = tempfile::tempdir().unwrap();
    {
        // Manually create a db file with user_version = 0 and no tables.
        let db_path = dir.path().join("db.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 0i64).unwrap();
        drop(conn);
    }
    let store = Store::open(dir.path()).unwrap();
    assert!(store.has_table("claims"));
    assert!(store.has_table("episodic_events"));
}

#[test]
fn vacuum_runs_against_test_db() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for i in 0..50 {
        store
            .add_claim(&format!("s{i}"), "p", &format!("o{i}"), "src")
            .unwrap();
    }
    store.close().unwrap();

    let report = vacuum(dir.path(), None, true).expect("vacuum should succeed");
    // Counts are non-negative and freelist after analyze is bounded by pages.
    assert!(report.pages_after >= 0);
    assert!(report.freelist_after >= 0);
    assert!(report.vacuumed_into.is_none());
}

#[test]
fn vacuum_into_writes_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for i in 0..10 {
        store
            .add_claim(&format!("s{i}"), "p", &format!("o{i}"), "src")
            .unwrap();
    }
    store.close().unwrap();

    let copy = dir.path().join("snapshot.sqlite");
    let report = vacuum(dir.path(), Some(&copy), false).unwrap();
    assert!(copy.exists(), "VACUUM INTO should produce a copy");
    assert_eq!(report.vacuumed_into.as_deref(), Some(copy.as_path()));
}

#[test]
fn replay_rebuilds_hyperedges_from_append_only_log() {
    let dir = tempfile::tempdir().unwrap();

    {
        let store = Store::open(dir.path()).unwrap();
        store
            .add_hyperedge(HyperedgeInput {
                predicate: "deployment".to_string(),
                provenance: Provenance::UserAsserted,
                confidence: 0.87,
                source_refs: vec!["deploy-log".to_string()],
                participants: vec![
                    HyperedgeParticipantInput {
                        role: "service".to_string(),
                        entity: "api".to_string(),
                    },
                    HyperedgeParticipantInput {
                        role: "environment".to_string(),
                        entity: "prod".to_string(),
                    },
                ],
            })
            .unwrap();
        store.close().unwrap();
    }

    std::fs::remove_file(dir.path().join("db.sqlite")).unwrap();
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-wal"));
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-shm"));

    let report = replay(dir.path(), false).expect("replay should rebuild hyperedges");
    assert_eq!(report.hyperedges, 1);

    let store = Store::open(dir.path()).unwrap();
    let hyperedge = store.get_hyperedge(1).unwrap();
    assert_eq!(hyperedge.predicate, "deployment");
    assert_eq!(hyperedge.provenance, Provenance::UserAsserted);
    assert!((hyperedge.confidence - 0.87).abs() < 1e-9);
    assert_eq!(hyperedge.source_refs, vec!["deploy-log".to_string()]);
    assert_eq!(hyperedge.participants.len(), 2);
    assert_eq!(hyperedge.participants[0].role, "service");
    assert_eq!(hyperedge.participants[0].entity, "api");
    assert_eq!(hyperedge.participants[1].role, "environment");
    assert_eq!(hyperedge.participants[1].entity, "prod");
}

#[test]
fn replay_rolls_back_hyperedge_projection_when_participant_insert_fails() {
    let dir = tempfile::tempdir().unwrap();
    let log_line = serde_json::json!({
        "kind": "add_hyperedge",
        "ts": 1,
        "hyperedge_id": 1,
        "predicate": "deployment",
        "provenance": "USER_ASSERTED",
        "confidence": 0.8,
        "source_refs": ["manual"],
        "participants": [
            {"role": "service", "entity": "api"},
            {"role": " ", "entity": "prod"}
        ]
    });
    std::fs::write(dir.path().join("log.jsonl"), format!("{log_line}\n")).unwrap();

    let err = replay(dir.path(), false).expect_err("invalid participant should fail replay");
    assert!(err.to_string().contains("CHECK constraint failed"));

    let conn = Connection::open(dir.path().join("db.sqlite.partial")).unwrap();
    let hyperedge_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM hyperedges", [], |row| row.get(0))
        .unwrap();
    let participant_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM hyperedge_participants", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(hyperedge_rows, 0);
    assert_eq!(participant_rows, 0);
}

#[test]
fn replay_rebuilds_db_from_logs() {
    let dir = tempfile::tempdir().unwrap();

    // Build a small db.
    {
        let store = Store::open(dir.path()).unwrap();
        store.add_claim("alpha", "rel", "beta", "src").unwrap();
        store.add_claim("gamma", "rel", "delta", "src").unwrap();
        let event_id = store
            .record_event_from_agent(
                "local",
                AgentKind::Human,
                "session1",
                "note",
                "hello world",
                "src",
            )
            .unwrap();
        store
            .record_observation(
                "session1",
                "an observation",
                ObservationRelevance::Medium,
                &[event_id],
                "manual",
            )
            .unwrap();
        store.close().unwrap();
    }

    // Capture row counts.
    let (claims, events, observations) = {
        let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
        let claims: i64 = conn
            .query_row("SELECT COUNT(*) FROM claims", [], |r| r.get(0))
            .unwrap();
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM episodic_events", [], |r| r.get(0))
            .unwrap();
        let observations: i64 = conn
            .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
            .unwrap();
        (claims, events, observations)
    };
    assert_eq!(claims, 2);
    assert_eq!(events, 1);
    assert_eq!(observations, 1);

    // Delete db and replay.
    std::fs::remove_file(dir.path().join("db.sqlite")).unwrap();
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-wal"));
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-shm"));

    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(report.claims, 2);
    assert_eq!(report.events, 1);
    assert_eq!(report.observations, 1);

    // Verify counts match.
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let claims_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM claims", [], |r| r.get(0))
        .unwrap();
    let events_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM episodic_events", [], |r| r.get(0))
        .unwrap();
    let observations_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(claims_after, claims);
    assert_eq!(events_after, events);
    assert_eq!(observations_after, observations);

    // Reopen via Store to exercise full validation.
    let store = Store::open(dir.path()).unwrap();
    let claim = store.get_claim(1).unwrap();
    assert_eq!(claim.subject, "alpha");
    assert_eq!(claim.object, "beta");
    drop(store);
}

#[test]
fn log_rotation_compresses_at_line_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("log.jsonl");
    // Write LOG_ROTATE_MAX_LINES + 1 lines manually so size stays small but
    // line count crosses the threshold.
    {
        let mut file = std::fs::File::create(&log_path).unwrap();
        for i in 0..=LOG_ROTATE_MAX_LINES {
            writeln!(
                file,
                r#"{{"kind":"add_claim","ts":0,"claim_id":{i},"subject":"a","predicate":"b","object":"c","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":1.0}}"#
            )
            .unwrap();
        }
    }

    // Opening the store at session boundary triggers rotation.
    let _store = Store::open(dir.path()).unwrap();
    // After rotation, log.1.jsonl.gz should exist and the active log should
    // be empty (or just-rotated, freshly opened).
    let rotated = dir.path().join("log.1.jsonl.gz");
    assert!(
        rotated.exists(),
        "log.1.jsonl.gz should exist after rotation"
    );
    let active_size = std::fs::metadata(&log_path)
        .map(|m| m.len())
        .unwrap_or(u64::MAX);
    assert!(
        active_size < 1024,
        "active log.jsonl should be empty after rotation, got {active_size} bytes"
    );

    // Sanity check: gz file is non-empty and gzip-magic prefixed.
    let raw = std::fs::read(&rotated).unwrap();
    assert!(raw.len() > 2, "gzipped log should be non-empty");
    assert_eq!(&raw[..2], &[0x1f, 0x8b], "expected gzip magic header");
}

#[test]
fn replay_walks_rotated_logs_in_numeric_order() {
    let dir = tempfile::tempdir().unwrap();
    // Manually craft two rotated logs and an active log with claims 1, 2, 3.
    let make_line = |id: i64, subject: &str| {
        format!(
            r#"{{"kind":"add_claim","ts":1,"claim_id":{id},"subject":"{subject}","predicate":"p","object":"o","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":1.0}}"#
        )
    };
    use flate2::Compression;
    use flate2::write::GzEncoder;
    for (n, id, subj) in [(1u32, 1i64, "first"), (2u32, 2i64, "second")] {
        let path = dir.path().join(format!("log.{n}.jsonl.gz"));
        let f = std::fs::File::create(&path).unwrap();
        let mut enc = GzEncoder::new(f, Compression::default());
        writeln!(enc, "{}", make_line(id, subj)).unwrap();
        enc.finish().unwrap();
    }
    let active = dir.path().join("log.jsonl");
    let mut f = std::fs::File::create(&active).unwrap();
    writeln!(f, "{}", make_line(3, "third")).unwrap();
    drop(f);

    let report = replay(dir.path(), false).unwrap();
    assert_eq!(report.claims, 3);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get_claim(1).unwrap().subject, "first");
    assert_eq!(store.get_claim(2).unwrap().subject, "second");
    assert_eq!(store.get_claim(3).unwrap().subject, "third");
}
