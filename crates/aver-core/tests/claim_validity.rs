//! ADR-0032: claim validity, explicit reverify and the evidence-since-
//! confirmation count. Invented garden vocabulary only.
//!
//! The count compares logged times, so those tests seed stores through log
//! lines with explicit timestamps and replay, like the recency tests.

use std::path::Path;

use aver_core::retrieval::HybridWeights;
use aver_core::vector::MockEmbeddingClient;
use aver_core::{
    AgentKind, ClaimStatus, ClaimWrite, Error, EventWrite, ReverifyWrite, Store, replay,
};
use rusqlite::Connection;
use serde_json::{Value, json};

const T0: i64 = 1_790_000_000;
const NOW: i64 = T0 + 1_000;

fn line(ts: i64, id: i64, subject: &str, predicate: &str, object: &str, scope: &str) -> Value {
    json!({
        "kind": "add_claim", "ts": ts, "claim_id": id, "subject": subject,
        "predicate": predicate, "object": object, "source": format!("src-{id}"),
        "agent_id": "local", "agent_kind": "HUMAN", "confidence": 0.95,
        "provenance": "USER_ASSERTED", "scope": scope,
    })
}

fn lesson(ts: i64, id: i64, subject: &str, scope: &str) -> Value {
    let mut value = line(ts, id, subject, "concerns", "topic:mulch", scope);
    value["context"] = json!("context:dry");
    value
}

/// An evidence item: in a context, concerning a topic, optionally closed.
fn item(
    id: i64,
    name: &str,
    context: &str,
    topic: &str,
    closed: Option<i64>,
    scope: &str,
) -> Vec<Value> {
    let mut lines = vec![
        line(T0 - 50, id, name, "in_context", context, scope),
        line(T0 - 50, id + 1, name, "concerns", topic, scope),
    ];
    if let Some(ts) = closed {
        let mut done = line(ts, id + 2, name, "closed_at", "2026-01-01", scope);
        done["object_kind"] = json!("literal");
        lines.push(done);
    }
    lines
}

fn reverify(ts: i64, claim_id: i64, evidence: &[&str]) -> Value {
    json!({
        "kind": "reverify_claim", "ts": ts, "claim_id": claim_id, "verified_at": ts,
        "agent_id": "review", "agent_kind": "LLM", "evidence": evidence,
    })
}

fn seeded(dir: &Path, lines: &[Value]) -> Store {
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(dir.join("log.jsonl"), text.join("\n") + "\n").unwrap();
    replay(dir, false).unwrap();
    Store::open(dir).unwrap()
}

fn count(store: &Store, id: i64, now: i64) -> Option<i64> {
    let claim = store.get_claim(id).unwrap();
    store
        .claim_confirmation(&claim, now)
        .unwrap()
        .evidence_since_confirmation
}

fn garden_log() -> Vec<Value> {
    let mut lines = vec![
        lesson(T0, 1, "lesson:shade", "a"),
        lesson(T0, 2, "lesson:sun", "a/b"),
        {
            let mut late = lesson(T0, 3, "lesson:late", "a");
            late["valid_from"] = json!(T0 + 25);
            late
        },
        line(T0, 4, "lesson:plain", "concerns", "topic:mulch", "a"),
        // A replay-scope hop must not widen the live lesson's topics.
        line(T0, 5, "lesson:shade", "concerns", "topic:z", "replay/r"),
    ];
    let dry = "context:dry";
    for (id, name, context, topic, closed, scope) in [
        (100, "plot:1", dry, "topic:mulch", Some(T0 + 10), "a"),
        (110, "plot:2", dry, "topic:mulch", Some(T0 + 20), "a/b"),
        (120, "plot:3", dry, "topic:mulch", None, "a"),
        (
            130,
            "plot:4",
            "context:wet",
            "topic:mulch",
            Some(T0 + 10),
            "a",
        ),
        (140, "plot:5", dry, "topic:other", Some(T0 + 10), "a"),
        (150, "plot:6", dry, "topic:mulch", Some(T0 + 10), "replay/r"),
        (160, "plot:7", dry, "topic:mulch", Some(T0), "a"),
        (170, "plot:8", dry, "topic:mulch", Some(NOW + 1), "a"),
        (180, "plot:9", dry, "lesson:shade", Some(T0 + 30), "a"),
        (190, "plot:11", dry, "topic:z", Some(T0 + 30), "a"),
        (210, "plot:12", dry, "topic:mulch", Some(T0 + 40), "a"),
    ] {
        lines.extend(item(id, name, context, topic, closed, scope));
    }
    // Each counted claim must itself be in the scope set.
    lines.extend(item(200, "plot:10", dry, "topic:mulch", Some(T0 + 10), "a"));
    let in_context = lines.len() - 3;
    lines[in_context]["scope"] = json!("replay/r");
    lines
}

#[test]
fn the_count_takes_closed_matching_items_in_the_scope_set_and_interval() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path(), &garden_log());
    // plot:1 (a), plot:2 (a/b), plot:9 (concerns the subject itself) and
    // plot:12. Not: open, other context, other topic, replay scope, closed
    // at the baseline, a claim outside the scope set, a replay-scope topic.
    assert_eq!(count(&store, 1, NOW), Some(4));
    // Upper bound: plot:8 closes at NOW + 1.
    assert_eq!(count(&store, 1, NOW + 1), Some(5));
    // A lesson in a/b sees only a/b.
    assert_eq!(count(&store, 2, NOW), Some(1));
    // The window start raises the baseline to T0 + 25: only plot:12.
    assert_eq!(count(&store, 3, NOW), Some(1));
    // No context, no count.
    assert_eq!(count(&store, 4, NOW), None);
    let confirmation = store
        .claim_confirmation(&store.get_claim(1).unwrap(), NOW)
        .unwrap();
    assert_eq!(confirmation.last_confirmed_at, T0);
    assert_eq!(confirmation.latest, None);
}

#[test]
fn an_explicit_confirmation_resets_the_count_and_a_folded_reassertion_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut lines = garden_log();
    lines.push(
        json!({"kind": "reverify_claim", "ts": T0 + 12, "claim_id": 1, "verified_at": T0 + 12}),
    );
    let store = seeded(dir.path(), &lines);
    assert_eq!(count(&store, 1, NOW), Some(4), "consolidation's reverify");
    assert_eq!(store.get_claim(1).unwrap().last_verified_at, Some(T0 + 12));
    drop(store);

    lines.push(reverify(T0 + 15, 1, &["plot:1"]));
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path(), &lines);
    // (T0 + 15, NOW] keeps plot:2 (T0 + 20), plot:9 (T0 + 30), plot:12.
    assert_eq!(count(&store, 1, NOW), Some(3));
    let confirmation = store
        .claim_confirmation(&store.get_claim(1).unwrap(), NOW)
        .unwrap();
    assert_eq!(confirmation.last_confirmed_at, T0 + 15);
    let latest = confirmation.latest.unwrap();
    assert_eq!(
        (
            latest.verified_at,
            latest.agent_id.as_str(),
            latest.agent_kind
        ),
        (T0 + 15, "review", AgentKind::Llm)
    );
    assert_eq!(latest.evidence, ["plot:1"]);
    // Replay again: same projection, same count.
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(count(&store, 1, NOW), Some(3));
}

#[test]
fn validity_round_trips_and_windows_need_a_context() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let write = || ClaimWrite::new("lesson:shade", "concerns", "topic:mulch", "synthetic");
    let id = store
        .add_claim(
            write()
                .context("context:dry")
                .valid_window(Some(T0), Some(T0 + 5))
                .scope("a"),
        )
        .unwrap();
    let claim = store.get_claim(id).unwrap();
    assert_eq!(
        (
            claim.context.as_deref(),
            claim.valid_from,
            claim.valid_until
        ),
        (Some("context:dry"), Some(T0), Some(T0 + 5))
    );
    for bad in [
        write().valid_window(Some(T0), None),
        write()
            .context("context:dry")
            .valid_window(Some(T0 + 1), Some(T0)),
        write().context("  "),
    ] {
        assert!(matches!(
            store.add_claim(bad),
            Err(Error::InvalidClaimValidity { .. })
        ));
    }
    assert!(matches!(
        store.add_claim(write().context("/home/alice/.ssh/id_ed25519")),
        Err(Error::Privacy(_))
    ));
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get_claim(id).unwrap().valid_until, Some(T0 + 5));
}

#[test]
fn context_partitions_duplicates_and_single_valued_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let write = |object: &str| ClaimWrite::new("bed:north", "prefers", object, "synthetic");
    let x = store
        .add_claim(write("shade").context("context:x"))
        .unwrap();
    let y = store
        .add_claim(write("shade").context("context:y"))
        .unwrap();
    let none = store.add_claim(write("shade")).unwrap();
    store.consolidate_report().unwrap();
    for id in [x, y, none] {
        assert_eq!(
            store.get_claim(id).unwrap().status,
            ClaimStatus::Active,
            "{id}"
        );
    }
    // Without a context, duplicates and single-valued replacement behave as before.
    let duplicate = store.add_claim(write("shade")).unwrap();
    let newer = store.add_claim(write("sun")).unwrap();
    store.consolidate_report().unwrap();
    assert_eq!(
        store.get_claim(duplicate).unwrap().status,
        ClaimStatus::Superseded
    );
    assert_eq!(
        store.get_claim(none).unwrap().status,
        ClaimStatus::Superseded
    );
    assert_eq!(store.get_claim(newer).unwrap().status, ClaimStatus::Active);
    // The contextual claims are untouched by the no-context replacement.
    for id in [x, y] {
        assert_eq!(
            store.get_claim(id).unwrap().status,
            ClaimStatus::Active,
            "{id}"
        );
    }
}

#[test]
fn reverify_records_its_writer_and_evidence_and_only_extends_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(EventWrite::new("s", "note", "Invented review.", "synthetic").scope("a/b"))
        .unwrap();
    let id = store
        .add_claim(
            ClaimWrite::new("lesson:shade", "concerns", "topic:mulch", "synthetic")
                .context("context:dry")
                .valid_window(Some(T0), Some(T0 + 5))
                .scope("a"),
        )
        .unwrap();
    let confirm = || {
        ReverifyWrite::new(id, [format!("event:{event}"), "plot:1".to_string()])
            .agent("review", AgentKind::Llm)
            .scope("a")
    };
    let at = store.reverify_claim(confirm().valid_until(T0 + 9)).unwrap();
    let claim = store.get_claim(id).unwrap();
    assert_eq!(claim.valid_until, Some(T0 + 9));
    assert_eq!(claim.last_verified_at, Some(at));
    let latest = store
        .claim_confirmation(&claim, at)
        .unwrap()
        .latest
        .unwrap();
    assert_eq!(latest.agent_id, "review");
    assert_eq!(
        latest.evidence,
        [format!("event:{event}"), "plot:1".to_string()]
    );
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let record: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(record["kind"], "reverify_claim");
    assert_eq!(record["agent_kind"], "LLM");
    assert_eq!(record["valid_until"], T0 + 9);

    let other = store
        .record_event(EventWrite::new("s", "note", "Elsewhere.", "synthetic").scope("c"))
        .unwrap();
    let plain = store
        .add_claim(
            ClaimWrite::new("lesson:plain", "concerns", "topic:mulch", "synthetic").scope("a"),
        )
        .unwrap();
    let log_after_confirm = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    for (write, expected) in [
        (confirm().scope("a/b"), "request scope"),
        (
            ReverifyWrite::new(id, Vec::<String>::new()).scope("a"),
            "at least one",
        ),
        (
            ReverifyWrite::new(id, [format!("event:{other}")]).scope("a"),
            "does not exist",
        ),
        (
            ReverifyWrite::new(id, ["event:x"]).scope("a"),
            "positive id",
        ),
        (
            ReverifyWrite::new(id, [format!("claim:{id}")]).scope("a"),
            "cite itself",
        ),
        (confirm().valid_until(T0 + 7), "only extends"),
        (
            ReverifyWrite::new(plain, ["plot:1"])
                .scope("a")
                .valid_until(T0),
            "needs a claim with a context",
        ),
    ] {
        let message = store.reverify_claim(write).unwrap_err().to_string();
        assert!(message.contains(expected), "{message}");
    }
    // The checks run inside the write transaction: a refusal appends
    // nothing and rolls back, so the next write still goes through.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap(),
        log_after_confirm
    );
    store.retire_claim(plain, "invented").unwrap();
    let message = store
        .reverify_claim(ReverifyWrite::new(plain, ["plot:1"]).scope("a"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("only ACTIVE"), "{message}");
    assert!(matches!(
        store.reverify_claim(ReverifyWrite::new(i64::MAX, ["plot:1"]).scope("a")),
        Err(Error::MissingClaim { .. })
    ));
    // A claim in a descendant scope is valid evidence.
    let below = store
        .add_claim(ClaimWrite::new("plot:1", "closed_at", "2026-01-01", "synthetic").scope("a/b"))
        .unwrap();
    store
        .reverify_claim(ReverifyWrite::new(id, [format!("claim:{below}")]).scope("a"))
        .unwrap();
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
}

#[test]
fn consolidation_reverify_keeps_its_bytes_and_adds_no_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let write = || ClaimWrite::new("bed:north", "uses", "mulch", "synthetic");
    store.add_claim(write()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    store.add_claim(write()).unwrap();
    store.consolidate_report().unwrap();
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let reverify: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["kind"] == "reverify_claim")
        .collect();
    assert_eq!(reverify.len(), 1);
    let mut keys: Vec<&String> = reverify[0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["claim_id", "kind", "ts", "verified_at"]);
    let rows: i64 = Connection::open(dir.path().join("db.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM claim_verifications", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

fn dump(dir: &Path) -> Vec<String> {
    let conn = Connection::open(dir.join("db.sqlite")).unwrap();
    let mut out = Vec::new();
    for sql in [
        "SELECT id, subject, predicate, object, status, last_verified_at, context, valid_from, valid_until, scope FROM claims ORDER BY id",
        "SELECT claim_id, ts, verified_at, agent_id, agent_kind, evidence FROM claim_verifications ORDER BY id",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                Ok((0..columns)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap();
        out.extend(rows.map(Result::unwrap));
    }
    out
}

#[test]
fn an_upgraded_store_equals_its_replay() {
    let dir = tempfile::tempdir().unwrap();
    let lines = vec![
        line(T0, 1, "bed:north", "uses", "mulch", "a"),
        line(T0 + 5, 2, "bed:north", "uses", "mulch", "a"),
        json!({"kind": "supersede_claims", "ts": T0 + 6, "claim_ids": [2]}),
        json!({"kind": "reverify_claim", "ts": T0 + 6, "claim_id": 1, "verified_at": T0 + 5}),
    ];
    seeded(dir.path(), &lines).close().unwrap();
    // Back to the schema before migration 0013, as an older binary left it.
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute_batch(
        "DROP INDEX claims_scope_context;
         DROP TRIGGER claims_validity_window_insert;
         DROP TRIGGER claims_validity_window_update;
         DROP TABLE claim_verifications;
         ALTER TABLE claims DROP COLUMN valid_until;
         ALTER TABLE claims DROP COLUMN valid_from;
         ALTER TABLE claims DROP COLUMN context;",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 122).unwrap();
    drop(conn);
    Store::open(dir.path()).unwrap().close().unwrap();
    let upgraded = dump(dir.path());
    assert!(upgraded.iter().any(|row| row.contains("Null")));
    replay(dir.path(), true).unwrap();
    assert_eq!(dump(dir.path()), upgraded);
}

#[test]
fn a_store_file_cannot_redefine_the_evidence_predicates() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ontology.json"),
        r#"{"version": 1, "predicates": [{"name": "closed_at", "parent": "relates_to"}]}"#,
    )
    .unwrap();
    let message = Store::open(dir.path()).err().unwrap().to_string();
    assert!(
        message.contains(r#"predicates[0] "closed_at": is a core predicate"#),
        "{message}"
    );
}

#[test]
fn evidence_ranking_is_opt_in_and_leaves_confidence_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut lines = vec![
        lesson(T0, 1, "lesson:shade", "a"),
        lesson(T0, 2, "lesson:shade", "a"),
    ];
    lines[1]["context"] = json!("context:wet");
    lines.extend(item(
        100,
        "plot:1",
        "context:dry",
        "topic:mulch",
        Some(T0 + 10),
        "a",
    ));
    let store = seeded(dir.path(), &lines);
    for id in [1, 2] {
        store
            .add_vector_chunk_with_embedding(id, "chunk", "mock", &[1.0, 0.0])
            .unwrap();
    }
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);
    let ids = |weights: HybridWeights| -> Vec<i64> {
        store
            .recall_hybrid_claims("lesson:shade", &client, 2, weights, NOW)
            .unwrap()
            .into_iter()
            .map(|claim| claim.id)
            .collect()
    };
    assert_eq!(ids(HybridWeights::default()), [1, 2]);
    let damped = HybridWeights::default()
        .with_evidence_half_life(1.0)
        .unwrap();
    assert_eq!(ids(damped), [2, 1]);
    assert_eq!(store.get_claim(1).unwrap().confidence, 0.95);
    assert!(
        HybridWeights::default()
            .with_evidence_half_life(0.0)
            .is_err()
    );
}
