//! Phase 1 exit criteria (ADR-0026): a store rebuilt by `replay` from its
//! JSONL alone is row-identical to the original, and two stores merge by
//! concatenating their logs.

use aver_core::{CandidateWrite, ClaimWrite, EventWrite, ObservationWrite};
use std::collections::BTreeMap;
use std::path::Path;

use aver_core::{
    AgentKind, ExtractorFact, HyperedgeInput, HyperedgeParticipantInput, ObservationRelevance,
    Provenance, Store, VECTOR_INDEX_DIM, replay, vector::MockEmbeddingClient,
};
use rusqlite::{Connection, types::Value};

const SERIES: [&str; 3] = ["log.jsonl", "events.jsonl", "observations.jsonl"];

/// Canonical dump: every ordinary table in `sqlite_master`, `SELECT <all
/// columns> ORDER BY <all columns>`, keyed by table name.
fn dump(db: &Path) -> BTreeMap<String, Vec<Vec<Value>>> {
    let conn = Connection::open(db).unwrap();
    let mut tables = conn
        .prepare(
            // `vector_index` is a vec0 virtual table whose shadow tables
            // (`vector_index_%`) hold opaque chunked blobs; its row count is
            // compared separately.
            "SELECT name FROM sqlite_master
              WHERE type = 'table'
                AND name NOT LIKE 'sqlite_%'
                AND name NOT LIKE 'vector_index%'
              ORDER BY name",
        )
        .unwrap();
    let names: Vec<String> = tables
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(tables);

    let mut out = BTreeMap::new();
    for table in names {
        let mut info = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns: Vec<String> = info
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(info);
        let select: Vec<String> = columns
            .iter()
            .map(|column| match (table.as_str(), column.as_str()) {
                // Seed/migration rows are stamped with `strftime('%s','now')`
                // at open time (ADR-0019 §4 "modulo migration-stamped
                // timestamps"); rows created by a logged write keep theirs.
                ("predicate_types", "created_at") => {
                    "CASE WHEN created_via IN ('seed', 'migration') THEN NULL ELSE created_at END"
                        .to_string()
                }
                ("predicate_alias", "created_at") => "NULL".to_string(),
                _ => column.clone(),
            })
            .collect();
        let sql = format!(
            "SELECT {} FROM {table} ORDER BY {}",
            select.join(", "),
            select.join(", ")
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let rows: Vec<Vec<Value>> = stmt
            .query_map([], |row| {
                (0..columns.len())
                    .map(|i| row.get::<_, Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.insert(table, rows);
    }
    out
}

fn user_version(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn count(db: &Path, table: &str) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn ids(db: &Path, table: &str) -> Vec<i64> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Exercise every `LogRecord` kind and the privacy rejection counter.
fn populate(dir: &Path, tag: &str) {
    let store = Store::open(dir).unwrap();
    let client = MockEmbeddingClient::new(vec![0.25; VECTOR_INDEX_DIM]);

    // Claims with each provenance; the first also auto-extends the ontology.
    let user = store
        .add_claim(
            ClaimWrite::new(
                format!("{tag}-svc"),
                format!("{tag}_brand_new_predicate"),
                "Config:db",
                "notes",
            )
            .scope("proj/aver"),
        )
        .unwrap();
    store
        .add_claim(ClaimWrite::from_agent(
            "parser",
            AgentKind::DeterministicParser,
            format!("{tag}-svc"),
            "depends_on",
            "Service:queue",
            "tree_sitter",
        ))
        .unwrap();
    let inferred = store
        .add_claim(ClaimWrite::from_agent(
            "llm",
            AgentKind::Llm,
            format!("{tag}-llm"),
            "prefers",
            "tabs",
            "chat",
        ))
        .unwrap();
    store
        .ingest_extractor_facts(
            "extractor",
            AgentKind::ExternalTool,
            "scan",
            "global",
            &[
                ExtractorFact::new(format!("{tag}-ambiguous"), "relates_to", "something")
                    .with_provenance(Provenance::Ambiguous),
            ],
        )
        .unwrap();
    // Duplicate and conflicting pairs for merge/supersede, a contradicted
    // claim for decay.
    store
        .add_claim(ClaimWrite::new("dup", "depends_on", "x", "src1"))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("dup", "depends_on", "x", "src2"))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("conflict", "prefers", "old", "src"))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("conflict", "prefers", "new", "src"))
        .unwrap();
    let weak = store
        .add_claim(ClaimWrite::new("weak", "depends_on", "y", "src"))
        .unwrap();
    store.add_contradiction(weak, "disputed").unwrap();
    store
        .contradict(
            user,
            "outdated",
            Some(ClaimWrite::new(
                format!("{tag}-svc"),
                "depends_on",
                "Config:db2",
                "review",
            )),
        )
        .unwrap();
    let report = store.consolidate_report().unwrap();
    assert!(report.merged >= 1 && report.superseded >= 2 && report.decayed >= 1);
    let future = time::OffsetDateTime::now_utc().unix_timestamp() + 100_000;
    assert!(store.decay_inferred_confidence_at(future, 1_000.0).unwrap() >= 1);
    store.retire_claim(inferred, "stale").unwrap();

    store
        .add_hyperedge(HyperedgeInput {
            predicate: "deployment".to_string(),
            provenance: Provenance::UserAsserted,
            confidence: 0.87,
            source_refs: vec!["deploy-log".to_string()],
            participants: vec![
                HyperedgeParticipantInput {
                    role: "service".to_string(),
                    entity: format!("Service:{tag}"),
                },
                HyperedgeParticipantInput {
                    role: "environment".to_string(),
                    entity: "prod".to_string(),
                },
            ],
        })
        .unwrap();

    // Events, observations (enough to prune), candidates promoted/rejected.
    let session = format!("{tag}-session");
    let mut events = Vec::new();
    for i in 0..3 {
        events.push(
            store
                .record_event(
                    EventWrite::new(&session, "note", format!("note {i}"), "src")
                        .scope("proj/aver"),
                )
                .unwrap(),
        );
    }
    for (i, event) in events.iter().enumerate() {
        store
            .record_observation(
                ObservationWrite::new(
                    &session,
                    format!("observed {i}"),
                    ObservationRelevance::Low,
                    &[*event],
                    "manual",
                )
                .scope("proj/aver"),
            )
            .unwrap();
    }
    assert_eq!(store.prune_observations(&session, 1).unwrap(), 2);
    let promoted = store
        .propose_candidate_claim(
            CandidateWrite::new(events[0], "alpha", "depends_on", "beta").scope("proj/aver"),
        )
        .unwrap();
    let rejected = store
        .propose_candidate_claim(CandidateWrite::new(
            events[1],
            "gamma",
            "depends_on",
            "delta",
        ))
        .unwrap();
    store.promote_candidate_claim(promoted).unwrap();
    store
        .reject_candidate_claim(rejected, "not useful")
        .unwrap();

    // Vector chunks: embedded at write, and plain then backfilled.
    store
        .add_embedded_vector_chunk_for_claim(user, "test-model", &client)
        .unwrap();
    store
        .add_vector_chunk(weak, "weak depends_on y", "test-model")
        .unwrap();
    assert_eq!(store.backfill_vector_embeddings(&client).unwrap(), 1);

    // A rejected write bumps the telemetry counter, which is not logged.
    store
        .add_claim(ClaimWrite::new(
            "key",
            "is",
            "-----BEGIN PRIVATE KEY-----",
            "src",
        ))
        .expect_err("privacy filter must reject the secret");
    store.close().unwrap();
}

fn copy_series(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for series in SERIES {
        std::fs::copy(from.join(series), to.join(series)).unwrap();
    }
}

#[test]
fn replay_rebuild_is_identical() {
    let live = tempfile::tempdir().unwrap();
    populate(live.path(), "a");

    let rebuilt = tempfile::tempdir().unwrap();
    copy_series(live.path(), rebuilt.path());
    replay(rebuilt.path(), false).expect("replay from JSONL alone");

    let live_db = live.path().join("db.sqlite");
    let rebuilt_db = rebuilt.path().join("db.sqlite");
    let mut expected = dump(&live_db);
    let mut actual = dump(&rebuilt_db);
    // Privacy rejection counters are telemetry about writes that never
    // reached the log; they are not part of the replayable state.
    assert_eq!(expected.remove("privacy_rejections").unwrap().len(), 1);
    assert_eq!(actual.remove("privacy_rejections").unwrap().len(), 0);

    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        actual.keys().collect::<Vec<_>>()
    );
    for (table, rows) in &expected {
        assert_eq!(&actual[table], rows, "table {table} diverged after replay");
        assert!(
            table == "entity_type_closure"
                || table == "predicate_closure"
                || table == "entity_types"
                || table == "predicate_alias"
                || !rows.is_empty(),
            "table {table} was not exercised"
        );
    }
    for table in [
        "claims",
        "hyperedges",
        "hyperedge_participants",
        "episodic_events",
        "observations",
        "observation_prune_markers",
        "candidate_claims",
        "contradictions",
        "vector_chunks",
        "entities",
        "ontology_extension_log",
    ] {
        assert!(!expected[table].is_empty(), "{table} must be exercised");
    }
    assert_eq!(user_version(&live_db), user_version(&rebuilt_db));
    assert_eq!(count(&live_db, "vector_index"), 2);
    assert_eq!(
        count(&live_db, "vector_index"),
        count(&rebuilt_db, "vector_index")
    );
}

#[test]
fn two_stores_merge_by_concatenating_logs() {
    let a = tempfile::tempdir().unwrap();
    populate(a.path(), "a");
    // Ids are time-ordered; make sure B's writes start in a later
    // millisecond than A's so the two generators cannot overlap.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let b = tempfile::tempdir().unwrap();
    populate(b.path(), "b");

    let merged = tempfile::tempdir().unwrap();
    for series in SERIES {
        let mut bytes = std::fs::read(a.path().join(series)).unwrap();
        bytes.extend(std::fs::read(b.path().join(series)).unwrap());
        std::fs::write(merged.path().join(series), bytes).unwrap();
    }
    replay(merged.path(), false).expect("concatenated logs must replay");

    let a_db = a.path().join("db.sqlite");
    let b_db = b.path().join("db.sqlite");
    let merged_db = merged.path().join("db.sqlite");
    for table in [
        "claims",
        "hyperedges",
        "episodic_events",
        "observations",
        "candidate_claims",
        "contradictions",
        "vector_chunks",
    ] {
        let mut expected: Vec<Value> = Vec::new();
        for db in [&a_db, &b_db] {
            let conn = Connection::open(db).unwrap();
            let mut stmt = conn
                .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
                .unwrap();
            expected.extend(
                stmt.query_map([], |r| r.get::<_, Value>(0))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        let conn = Connection::open(&merged_db).unwrap();
        let mut stmt = conn
            .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
            .unwrap();
        let mut actual: Vec<Value> = stmt
            .query_map([], |r| r.get::<_, Value>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut expected_sorted = expected.clone();
        let key = |v: &Value| match v {
            Value::Integer(i) => i.to_string(),
            Value::Text(t) => t.clone(),
            other => format!("{other:?}"),
        };
        expected_sorted.sort_by_key(key);
        actual.sort_by_key(key);
        assert_eq!(
            actual, expected_sorted,
            "{table}: every id of A and B present"
        );
        assert_eq!(
            count(&merged_db, table),
            count(&a_db, table) + count(&b_db, table),
            "{table} total"
        );
    }
    // Both stores wrote the same subject/predicate pair ("dup" depends_on
    // "x" twice each); all four claims survive with their original ids
    // because replay never consolidates.
    let dup_ids: Vec<i64> = {
        let conn = Connection::open(&merged_db).unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM claims WHERE subject = 'dup' ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(dup_ids.len(), 4);
    let mut a_and_b = ids(&a_db, "claims");
    a_and_b.extend(ids(&b_db, "claims"));
    assert!(dup_ids.iter().all(|id| a_and_b.contains(id)));
}
