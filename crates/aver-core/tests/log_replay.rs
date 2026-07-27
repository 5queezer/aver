//! Log-first replay completeness: scope round-trip, lifecycle records
//! (retire/contradict/candidate/supersede/decay/merge), shared entity
//! typing, promote-time validation ordering, id allocation under
//! concurrency, lenient replay, and the vector-chunk derived-projection
//! boundary.

use std::fmt::Write as _;
use std::path::Path;

use aver_core::{
    AgentKind, ClaimStatus, HyperedgeInput, HyperedgeParticipantInput, ObservationRelevance,
    Provenance, ReplayMode, Store, replay, replay_with_mode, vector::MockEmbeddingClient,
};
use rusqlite::Connection;

fn delete_db(dir: &Path) {
    std::fs::remove_file(dir.join("db.sqlite")).unwrap();
    let _ = std::fs::remove_file(dir.join("db.sqlite-wal"));
    let _ = std::fs::remove_file(dir.join("db.sqlite-shm"));
}

/// (id, subject, predicate, object, provenance, confidence, status,
/// source_refs, scope) for every claim, in id order.
type ClaimRows = Vec<(
    i64,
    String,
    String,
    String,
    String,
    f64,
    String,
    String,
    String,
)>;

fn claims_rows(dir: &Path) -> ClaimRows {
    let conn = Connection::open(dir.join("db.sqlite")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs, scope
               FROM claims ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn assert_claim_rows_eq(live: &ClaimRows, replayed: &ClaimRows) {
    assert_eq!(live.len(), replayed.len(), "claim row count diverged");
    for (live_row, replayed_row) in live.iter().zip(replayed.iter()) {
        assert_eq!(live_row.0, replayed_row.0, "id");
        assert_eq!(live_row.1, replayed_row.1, "subject");
        assert_eq!(live_row.2, replayed_row.2, "predicate");
        assert_eq!(live_row.3, replayed_row.3, "object");
        assert_eq!(live_row.4, replayed_row.4, "provenance");
        assert!(
            (live_row.5 - replayed_row.5).abs() < 1e-9,
            "confidence diverged: {} vs {}",
            live_row.5,
            replayed_row.5
        );
        assert_eq!(live_row.6, replayed_row.6, "status");
        assert_eq!(live_row.7, replayed_row.7, "source_refs");
        assert_eq!(live_row.8, replayed_row.8, "scope");
    }
}

#[test]
fn replay_preserves_scope_for_claims_events_observations() {
    let dir = tempfile::tempdir().unwrap();
    let (claim_id, event_id, observation_id) = {
        let store = Store::open(dir.path()).unwrap();
        let claim_id = store
            .add_claim_with_scope("alpha", "depends_on", "beta", "src", "project/sub")
            .unwrap();
        let event_id = store
            .record_event_with_scope("s1", "note", "payload", "src", "project/sub")
            .unwrap();
        let observation_id = store
            .record_observation_with_scope(
                "s1",
                "an observation",
                ObservationRelevance::High,
                &[event_id],
                "manual",
                "project/sub",
            )
            .unwrap();
        store.close().unwrap();
        (claim_id, event_id, observation_id)
    };

    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(
        (report.claims, report.events, report.observations),
        (1, 1, 1)
    );

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get_claim(claim_id).unwrap().scope, "project/sub");
    assert_eq!(store.get_event(event_id).unwrap().scope, "project/sub");
    assert_eq!(
        store.get_observation(&observation_id).unwrap().scope,
        "project/sub"
    );
}

#[test]
fn replay_preserves_retired_claim_status_and_reason() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let claim_id = store
            .add_claim("alpha", "depends_on", "beta", "src")
            .unwrap();
        store.retire_claim(claim_id, "superseded by docs").unwrap();
        store.close().unwrap();
    }
    let live = claims_rows(dir.path());

    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(report.lifecycle, 1, "expected one retire_claim record");

    let replayed = claims_rows(dir.path());
    assert_claim_rows_eq(&live, &replayed);
    let store = Store::open(dir.path()).unwrap();
    let claim = store.get_claim(1).unwrap();
    assert_eq!(claim.status, ClaimStatus::Invalidated);
    assert!(
        claim
            .source_refs
            .iter()
            .any(|r| r == "retired:superseded by docs"),
        "retired:<reason> marker must survive replay: {:?}",
        claim.source_refs
    );
}

#[test]
fn replay_preserves_contradictions() {
    let dir = tempfile::tempdir().unwrap();
    let (claim_a, claim_b) = {
        let store = Store::open(dir.path()).unwrap();
        let claim_a = store
            .add_claim("alpha", "depends_on", "beta", "src")
            .unwrap();
        // contradict with a replacement claim + a plain audit record.
        let record = store
            .contradict(
                claim_a,
                "outdated dependency",
                Some(aver_core::NewClaim {
                    subject: "alpha",
                    predicate: "depends_on",
                    object: "gamma",
                    source: "review",
                }),
            )
            .unwrap();
        let claim_b = record.new_claim_id.unwrap();
        store.add_contradiction(claim_a, "second look").unwrap();
        store.close().unwrap();
        (claim_a, claim_b)
    };

    delete_db(dir.path());
    replay(dir.path(), false).expect("replay should succeed");

    let store = Store::open(dir.path()).unwrap();
    let records = store.list_contradictions(claim_a).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].reason, "outdated dependency");
    assert_eq!(records[0].new_claim_id, Some(claim_b));
    assert_eq!(records[1].reason, "second look");
    assert_eq!(records[1].new_claim_id, None);
}

#[test]
fn replay_preserves_candidate_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let (promoted_id, rejected_id, claim_id) = {
        let store = Store::open(dir.path()).unwrap();
        let event_id = store.record_event("s1", "note", "payload", "src").unwrap();
        let promoted_id = store
            .propose_candidate_claim(event_id, "alpha", "depends_on", "beta")
            .unwrap();
        let rejected_id = store
            .propose_candidate_claim(event_id, "gamma", "depends_on", "delta")
            .unwrap();
        let claim_id = store.promote_candidate_claim(promoted_id).unwrap();
        store
            .reject_candidate_claim(rejected_id, "not useful")
            .unwrap();
        store.close().unwrap();
        (promoted_id, rejected_id, claim_id)
    };

    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(
        report.lifecycle, 4,
        "two proposals + one promotion + one rejection"
    );

    let store = Store::open(dir.path()).unwrap();
    let promoted = store.get_candidate_claim(promoted_id).unwrap();
    assert_eq!(promoted.status, "PROMOTED");
    assert_eq!(promoted.promoted_claim_id, Some(claim_id));
    let rejected = store.get_candidate_claim(rejected_id).unwrap();
    assert_eq!(rejected.status, "REJECTED");
    assert_eq!(rejected.rejection_reason.as_deref(), Some("not useful"));
    assert_eq!(store.get_claim(claim_id).unwrap().subject, "alpha");
}

#[test]
fn replay_preserves_consolidation_outcome() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        // Duplicate pair -> merge + supersede of the non-survivor.
        store.add_claim("dup", "depends_on", "x", "src1").unwrap();
        store.add_claim("dup", "depends_on", "x", "src2").unwrap();
        // Conflict pair -> the older claim is superseded by the newer.
        store
            .add_claim("conflict", "depends_on", "old", "src")
            .unwrap();
        store
            .add_claim("conflict", "depends_on", "new", "src")
            .unwrap();
        // Contradicted claim -> decayed by consolidation.
        let contradicted = store.add_claim("weak", "depends_on", "y", "src").unwrap();
        store.add_contradiction(contradicted, "disputed").unwrap();
        // INFERRED claim -> explicit exponential decay step.
        store
            .add_claim_from_agent(
                "agent-llm",
                AgentKind::Llm,
                "inferred",
                "depends_on",
                "z",
                "chat",
            )
            .unwrap();
        let report = store.consolidate_report().unwrap();
        assert!(report.superseded >= 2 && report.decayed >= 1);
        let future = time::OffsetDateTime::now_utc().unix_timestamp() + 100_000;
        store.decay_inferred_confidence_at(future, 1_000.0).unwrap();
        store.close().unwrap();
    }
    let live = claims_rows(dir.path());

    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert!(report.lifecycle >= 3, "merge/decay/supersede records");

    let replayed = claims_rows(dir.path());
    assert_claim_rows_eq(&live, &replayed);
}

#[test]
fn promote_rejects_unknown_predicate_before_logging() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let event_id = store
            .record_event_from_agent("agent-llm", AgentKind::Llm, "s1", "note", "payload", "src")
            .unwrap();
        let candidate_id = store
            .propose_candidate_claim(event_id, "alpha", "definitely_not_a_predicate", "beta")
            .unwrap();
        let err = store
            .promote_candidate_claim(candidate_id)
            .expect_err("INFERRED candidate with unknown predicate must be rejected");
        assert!(
            err.to_string().contains("unknown predicate"),
            "unexpected error: {err}"
        );
        // The candidate stays staged; nothing was promoted.
        let candidate = store.get_candidate_claim(candidate_id).unwrap();
        assert_eq!(candidate.status, "PENDING");
        store.close().unwrap();
    }

    // No poisoned add_claim line may reference the rejected predicate, in
    // any log file. (A failed first promote leaves no claims behind, so
    // log.jsonl itself may not exist yet.)
    let mut claim_lines = Vec::new();
    for path in [
        dir.path().join("log.jsonl"),
        dir.path().join("agents/agent-llm/log.jsonl"),
    ] {
        if let Ok(contents) = std::fs::read_to_string(&path) {
            claim_lines.extend(contents.lines().map(str::to_owned).collect::<Vec<_>>());
        }
    }
    for line in &claim_lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        if value["kind"] == "add_claim" {
            assert_ne!(
                value["predicate"], "definitely_not_a_predicate",
                "promote appended an add_claim line before ontology validation"
            );
        }
    }
    // And replay of the (clean) log succeeds strictly.
    delete_db(dir.path());
    replay(dir.path(), false).expect("replay should succeed");
}

#[test]
fn replay_preserves_promoted_claim_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let claim_id = {
        let store = Store::open(dir.path()).unwrap();
        // HUMAN-authored event, but the candidate keeps the INFERRED default
        // provenance — replay must not derive USER_ASSERTED from agent_kind.
        let event_id = store.record_event("s1", "note", "payload", "src").unwrap();
        let candidate_id = store
            .propose_candidate_claim(event_id, "alpha", "depends_on", "beta")
            .unwrap();
        let claim_id = store.promote_candidate_claim(candidate_id).unwrap();
        store.close().unwrap();
        claim_id
    };

    delete_db(dir.path());
    replay(dir.path(), false).expect("replay should succeed");

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.get_claim(claim_id).unwrap().provenance,
        Provenance::Inferred,
        "promoted candidate provenance must round-trip explicitly"
    );
}

#[test]
fn replay_entity_typing_matches_live() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store
            .add_claim("Service:api", "depends_on", "Config:db", "src")
            .unwrap();
        store
            .add_claim("plain-thing", "depends_on", "x", "src")
            .unwrap();
        store.close().unwrap();
    }
    let entity_rows = |dir: &Path| {
        let conn = Connection::open(dir.join("db.sqlite")).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT entities.name, entity_types.name, entities.requires_review
                   FROM entities
                   JOIN entity_types ON entity_types.id = entities.type_id
                  ORDER BY entities.name",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let live = entity_rows(dir.path());

    delete_db(dir.path());
    replay(dir.path(), false).expect("replay should succeed");

    let replayed = entity_rows(dir.path());
    assert_eq!(live, replayed, "entity classification must survive replay");
    assert!(
        live.iter()
            .any(|(name, ty, review)| name == "Service:api" && ty == "Service" && *review == 0),
        "prefix-typed entity expected: {live:?}"
    );
    assert!(
        live.iter()
            .any(|(name, ty, review)| name == "plain-thing" && ty == "Thing" && *review == 1),
        "Thing fallback must keep requires_review=1: {live:?}"
    );
}

#[test]
fn concurrent_writers_allocate_unique_claim_ids() {
    let dir = tempfile::tempdir().unwrap();
    // First store creates schema + logs; the second joins the same dir.
    let first = Store::open(dir.path()).unwrap();
    first.close().unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for thread_index in 0..2 {
        let memory_dir = dir.path().to_path_buf();
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = Store::open(&memory_dir).unwrap();
            barrier.wait();
            for i in 0..25 {
                store
                    .add_claim(
                        &format!("t{thread_index}-subject-{i}"),
                        "depends_on",
                        "object",
                        "race-test",
                    )
                    .unwrap();
            }
            store.close().unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    // 50 claims with dense, unique ids — no allocation race.
    let rows = claims_rows(dir.path());
    assert_eq!(rows.len(), 50);
    let ids: std::collections::BTreeSet<i64> = rows.iter().map(|row| row.0).collect();
    assert_eq!(ids.len(), 50, "claim ids must be unique");
    assert_eq!(*ids.iter().next().unwrap(), 1);
    assert_eq!(*ids.iter().next_back().unwrap(), 50);

    // The log carries exactly those 50 unique claim ids.
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let logged_ids: std::collections::BTreeSet<i64> = log
        .lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["kind"], "add_claim");
            value["claim_id"].as_i64().unwrap()
        })
        .collect();
    assert_eq!(logged_ids.len(), 50, "log must not contain duplicate ids");

    // And the log replays cleanly end to end.
    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(report.claims, 50);
}

#[test]
fn lenient_replay_quarantines_bad_lines() {
    let dir = tempfile::tempdir().unwrap();
    let valid_line = |id: i64, subject: &str| {
        format!(
            r#"{{"kind":"add_claim","ts":1,"claim_id":{id},"subject":"{subject}","predicate":"depends_on","object":"o","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":0.95,"provenance":"USER_ASSERTED","scope":"global"}}"#
        )
    };
    let mut log = String::new();
    writeln!(log, "{}", valid_line(1, "good-one")).unwrap();
    writeln!(
        log,
        r#"{{"kind":"add_claim","ts":1,"claim_id": broken json"#
    )
    .unwrap();
    writeln!(log, r#"{{"kind":"no_such_kind","ts":1}}"#).unwrap();
    // Confidence outside the 0..=1 CHECK range: projection INSERT fails
    // after entity and ontology helpers have run.
    writeln!(
        log,
        r#"{{"kind":"add_claim","ts":1,"claim_id":2,"subject":"bad","predicate":"quarantined_predicate","object":"o","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":5.0}}"#
    )
    .unwrap();
    // Lifecycle record referencing a claim that does not exist.
    writeln!(
        log,
        r#"{{"kind":"retire_claim","ts":2,"claim_id":999,"reason":"gone"}}"#
    )
    .unwrap();
    writeln!(log, "{}", valid_line(3, "good-two")).unwrap();
    std::fs::write(dir.path().join("log.jsonl"), log).unwrap();

    // Strict mode still aborts on the first bad line (ADR-0019 §4 default).
    replay(dir.path(), false).expect_err("strict replay must fail on a poisoned line");

    // Lenient mode quarantines the bad lines and applies the rest.
    let report = replay_with_mode(dir.path(), false, ReplayMode::Lenient)
        .expect("lenient replay should succeed");
    assert_eq!(report.claims, 2);
    assert_eq!(report.quarantined.len(), 4);
    let quarantined_lines: Vec<usize> = report.quarantined.iter().map(|entry| entry.line).collect();
    assert_eq!(quarantined_lines, vec![2, 3, 4, 5]);
    assert!(
        report
            .quarantined
            .iter()
            .all(|entry| entry.path.ends_with("log.jsonl") && !entry.error.is_empty()),
        "quarantine diagnostics must carry path + error: {:?}",
        report.quarantined
    );

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get_claim(1).unwrap().subject, "good-one");
    assert_eq!(store.get_claim(3).unwrap().subject, "good-two");
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let entity_debris: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE name = 'bad'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(entity_debris, 0, "quarantined line left entity debris");
    let ontology_debris: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM predicate_types WHERE name = 'quarantined_predicate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ontology_debris, 0, "quarantined line left ontology debris");
}

#[test]
fn vector_chunks_are_rebuildable_after_replay() {
    let dir = tempfile::tempdir().unwrap();
    let claim_id = {
        let store = Store::open(dir.path()).unwrap();
        let claim_id = store
            .add_claim("alpha", "depends_on", "beta", "src")
            .unwrap();
        let client = MockEmbeddingClient::new(vec![0.5; 8]);
        store
            .add_embedded_vector_chunk_for_claim(claim_id, "test-model", &client)
            .unwrap();
        assert_eq!(
            store.list_vector_chunks_for_claim(claim_id).unwrap().len(),
            1
        );
        store.close().unwrap();
        claim_id
    };

    // Vector chunks are a derived projection: replay rebuilds claims and
    // leaves the chunk tables empty rather than bloating the log with
    // embedding arrays.
    delete_db(dir.path());
    replay(dir.path(), false).expect("replay should succeed");

    let store = Store::open(dir.path()).unwrap();
    assert!(
        store
            .list_vector_chunks_for_claim(claim_id)
            .unwrap()
            .is_empty(),
        "replay must not resurrect vector chunks"
    );
    // Recall keeps working over the rebuilt claims, and the chunks can be
    // re-derived from claim text with the same embedding client.
    assert!(!store.recall_text("alpha").unwrap().is_empty());
    let client = MockEmbeddingClient::new(vec![0.5; 8]);
    store
        .add_embedded_vector_chunk_for_claim(claim_id, "test-model", &client)
        .unwrap();
    assert_eq!(
        store.list_vector_chunks_for_claim(claim_id).unwrap().len(),
        1
    );
}

#[test]
fn old_log_lines_without_scope_and_provenance_still_replay() {
    let dir = tempfile::tempdir().unwrap();
    // Line format from before the scope/provenance fields were logged.
    std::fs::write(
        dir.path().join("log.jsonl"),
        r#"{"kind":"add_claim","ts":1,"claim_id":1,"subject":"alpha","predicate":"depends_on","object":"beta","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":0.95}"#
            .to_string()
            + "\n",
    )
    .unwrap();

    let report = replay(dir.path(), false).expect("legacy lines must replay");
    assert_eq!(report.claims, 1);
    let store = Store::open(dir.path()).unwrap();
    let claim = store.get_claim(1).unwrap();
    assert_eq!(claim.scope, "global");
    assert_eq!(claim.provenance, Provenance::UserAsserted);
}

#[test]
fn failed_claim_append_does_not_extend_ontology() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("log.jsonl")).unwrap();
    store
        .add_claim("alpha", "new_claim_predicate", "beta", "src")
        .expect_err("a directory at log.jsonl must make append fail");

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let predicate_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM predicate_types WHERE name = 'new_claim_predicate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(predicate_count, 0, "failed append mutated predicate_types");
    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ontology_extension_log WHERE predicate = 'new_claim_predicate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 0, "failed append emitted ontology audit state");
}

#[test]
fn failed_hyperedge_append_does_not_extend_ontology() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("log.jsonl")).unwrap();
    store
        .add_hyperedge(HyperedgeInput {
            predicate: "new_hyperedge_predicate".to_string(),
            provenance: Provenance::UserAsserted,
            confidence: 0.9,
            source_refs: vec!["src".to_string()],
            participants: vec![HyperedgeParticipantInput {
                role: "member".to_string(),
                entity: "alpha".to_string(),
            }],
        })
        .expect_err("a directory at log.jsonl must make append fail");

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let predicate_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM predicate_types WHERE name = 'new_hyperedge_predicate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(predicate_count, 0, "failed append mutated predicate_types");
}

#[test]
fn rejecting_candidate_twice_with_same_reason_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store.record_event("s1", "note", "payload", "src").unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "alpha", "depends_on", "beta")
        .unwrap();
    store
        .reject_candidate_claim(candidate_id, "duplicate")
        .unwrap();
    store
        .reject_candidate_claim(candidate_id, "duplicate")
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
    let rejection_count = log
        .lines()
        .filter(|line| line.contains("\"kind\":\"reject_candidate_claim\""))
        .count();
    assert_eq!(
        rejection_count, 1,
        "identical retry appended another record"
    );
}

#[test]
fn rejecting_candidate_twice_with_different_reason_is_rejected_without_logging() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store.record_event("s1", "note", "payload", "src").unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "alpha", "depends_on", "beta")
        .unwrap();
    store
        .reject_candidate_claim(candidate_id, "duplicate")
        .unwrap();
    store
        .reject_candidate_claim(candidate_id, "unsupported")
        .expect_err("a conflicting rejection must fail");

    let candidate = store.get_candidate_claim(candidate_id).unwrap();
    assert_eq!(candidate.rejection_reason.as_deref(), Some("duplicate"));
    let log = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
    assert!(
        !log.contains("unsupported"),
        "conflicting retry poisoned log"
    );
}

#[test]
fn strict_replay_rejects_conflicting_candidate_terminal_transitions() {
    let cases = [
        (
            "reject-then-promote",
            r#"{"kind":"reject_candidate_claim","ts":3,"candidate_id":1,"reason":"no"}
{"kind":"promote_candidate_claim","ts":4,"candidate_id":1,"claim_id":1}"#,
        ),
        (
            "promote-then-reject",
            r#"{"kind":"promote_candidate_claim","ts":3,"candidate_id":1,"claim_id":1}
{"kind":"reject_candidate_claim","ts":4,"candidate_id":1,"reason":"no"}"#,
        ),
    ];
    for (name, transitions) in cases {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("log.jsonl"),
            r#"{"kind":"add_claim","ts":1,"claim_id":1,"subject":"alpha","predicate":"depends_on","object":"beta","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":0.95}
"#,
        )
        .unwrap();
        let events = format!(
            r#"{{"kind":"record_event","ts":1,"event_id":1,"session_id":"s","event_kind":"note","payload":"p","source":"s","agent_id":"local","agent_kind":"HUMAN"}}
{{"kind":"propose_candidate_claim","ts":2,"candidate_id":1,"event_id":1,"subject":"alpha","predicate":"depends_on","object":"beta"}}
{transitions}
"#
        );
        std::fs::write(dir.path().join("events.jsonl"), events).unwrap();
        assert!(
            replay(dir.path(), false).is_err(),
            "{name} must fail strict replay"
        );
    }
}

#[test]
fn strict_replay_rejects_lifecycle_records_with_missing_claims() {
    let records = [
        r#"{"kind":"supersede_claims","ts":2,"claim_ids":[999]}"#,
        r#"{"kind":"decay_confidence","ts":2,"changes":[{"claim_id":999,"confidence":0.4}]}"#,
        r#"{"kind":"merge_source_refs","ts":2,"claim_id":999,"source_refs":["s"],"promote":false}"#,
    ];
    for record in records {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("log.jsonl"), format!("{record}\n")).unwrap();
        replay(dir.path(), false).expect_err("missing lifecycle reference must fail strict replay");
    }
}
