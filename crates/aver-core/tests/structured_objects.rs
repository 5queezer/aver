use aver_core::{ClaimWrite, ObjectKind, Provenance, Store, replay};
use rusqlite::Connection;

fn entity_exists(dir: &std::path::Path, name: &str) -> bool {
    Connection::open(dir.join("db.sqlite"))
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM entities WHERE name = ?1)",
            [name],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn objects_keep_text_but_prose_and_numbers_are_not_entities() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for object in [
        "SKU-4711",
        "decision:invented-42",
        "event:123",
        "src/main.rs",
        "https://example.test/a",
        "New York",
        "Rust 1.95",
        "https://example.test/search?",
        "https://example.test/a!",
        "./src/file.",
        ".",
        "..",
    ] {
        let id = store
            .add_claim(ClaimWrite::new("sample", "concerns", object, "synthetic"))
            .unwrap();
        assert_eq!(
            store.get_claim(id).unwrap().object_kind,
            ObjectKind::Entity,
            "{object}"
        );
        assert!(entity_exists(dir.path(), object));
    }
    for object in [
        "Price crossed the invented threshold.",
        "one\ntwo",
        "This object contains more than eight separate words without punctuation",
        "-0.18",
        "42",
        "true",
        "\"short text\"",
    ] {
        let id = store
            .add_claim(ClaimWrite::new("sample", "concerns", object, "synthetic"))
            .unwrap();
        let claim = store.get_claim(id).unwrap();
        assert_eq!(claim.object, object);
        assert_eq!(claim.object_kind, ObjectKind::Literal, "{object}");
        assert!(!entity_exists(dir.path(), object));
        let graph = store.expand("sample", 2, None).unwrap();
        assert!(!graph.nodes.contains(&object.to_string()));
        assert!(graph.edges.iter().any(|c| c.id == id));
    }
    let id = store
        .add_claim(
            ClaimWrite::new("sample", "concerns", "short label", "synthetic")
                .object_kind(ObjectKind::Literal),
        )
        .unwrap();
    assert_eq!(
        store.get_claim(id).unwrap().object_kind,
        ObjectKind::Literal
    );
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
    assert_eq!(
        Store::open(dir.path())
            .unwrap()
            .get_claim(id)
            .unwrap()
            .object_kind,
        ObjectKind::Literal
    );
}

#[test]
fn explicit_entity_cannot_turn_prose_into_a_node() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(
        store
            .add_claim(
                ClaimWrite::new(
                    "sample",
                    "concerns",
                    "A full invented sentence.",
                    "synthetic"
                )
                .object_kind(ObjectKind::Entity)
            )
            .is_err()
    );
    assert!(!dir.path().join("log.jsonl").exists());
}

#[test]
fn legacy_cleanup_is_logged_idempotent_and_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = concat!(
        r#"{"kind":"add_claim","ts":1,"claim_id":1,"subject":"sample","predicate":"concerns","object":"An invented complete sentence.","source":"synthetic","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9,"provenance":"EXTRACTED","scope":"global"}"#,
        "\n"
    );
    std::fs::write(dir.path().join("log.jsonl"), legacy).unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(entity_exists(dir.path(), "An invented complete sentence."));
    let before = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    let preview = store.preview_literal_migration().unwrap();
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), before);
    let report = store.migrate_literals().unwrap();
    assert_eq!(report, preview);
    assert_eq!(report.claims_reclassified, 1);
    assert_eq!(report.entities_removed, 1);
    assert_eq!(
        store.get_claim(1).unwrap().provenance,
        Provenance::Extracted
    );
    assert_eq!(store.get_claim(1).unwrap().confidence, 0.9);
    let logged = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    assert!(logged.starts_with(legacy.as_bytes()));
    assert_eq!(store.migrate_literals().unwrap().claims_reclassified, 0);
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), logged);
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get_claim(1).unwrap().object_kind, ObjectKind::Literal);
    assert!(!entity_exists(dir.path(), "An invented complete sentence."));
    assert_eq!(store.migrate_literals().unwrap().claims_reclassified, 0);
}

#[test]
fn inferred_confidence_cap_applies_only_to_new_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let id = store
        .add_claim(
            ClaimWrite::new("sample", "concerns", "judgment", "synthetic")
                .provenance(Provenance::Inferred)
                .confidence(0.99),
        )
        .unwrap();
    assert_eq!(store.get_claim(id).unwrap().confidence, 0.75);
}

#[test]
fn cleanup_preserves_subject_and_hyperedge_entities_and_recovers_unapplied_record() {
    let dir = tempfile::tempdir().unwrap();
    let sentence = "An invented sentence.";
    let legacy = serde_json::json!({"kind":"add_claim","ts":1,"claim_id":1,"subject":sentence,"predicate":"concerns","object":sentence,"source":"synthetic","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9,"provenance":"EXTRACTED","scope":"global"});
    std::fs::write(dir.path().join("log.jsonl"), format!("{legacy}\n")).unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    let before = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    let preview = store.preview_literal_migration().unwrap();
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), before);
    let report = store.migrate_literals().unwrap();
    assert_eq!(report, preview);
    assert_eq!(report.claims_reclassified, 1);
    assert_eq!(report.entities_removed, 0);
    assert!(entity_exists(dir.path(), sentence));
    let protected = "Another invented sentence.";
    store.close().unwrap();
    let hyperedge = serde_json::json!({"kind":"add_hyperedge","ts":2,"hyperedge_id":1,"predicate":"concerns","provenance":"USER_ASSERTED","confidence":0.8,"source_refs":["synthetic"],"participants":[{"role":"context","entity":protected}]});
    let mut log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    log.push_str(&format!("{hyperedge}\n"));
    std::fs::write(dir.path().join("log.jsonl"), log).unwrap();
    replay(dir.path(), true).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.migrate_literals().unwrap().entities_removed, 0);
    assert!(entity_exists(dir.path(), protected));
    store.close().unwrap();
    // Simulate a crash after a durable migration record but before apply.
    use std::io::Write;
    let record = serde_json::json!({"kind":"migrate_literals","ts":2,"claim_ids":[1],"entity_names":[sentence,protected]});
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("log.jsonl"))
            .unwrap(),
        "{record}"
    )
    .unwrap();
    for _ in 0..2 {
        replay(dir.path(), true).unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.get_claim(1).unwrap().object_kind, ObjectKind::Literal);
        assert!(entity_exists(dir.path(), sentence));
        assert!(entity_exists(dir.path(), protected));
        assert_eq!(store.migrate_literals().unwrap().claims_reclassified, 0);
        store.close().unwrap();
    }
}

#[test]
fn consolidation_preserves_entity_and_literal_identity_with_the_same_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let entity = store
        .add_claim(ClaimWrite::new(
            "sample",
            "concerns",
            "short label",
            "entity-source",
        ))
        .unwrap();
    let literal = store
        .add_claim(
            ClaimWrite::new("sample", "concerns", "short label", "literal-source")
                .object_kind(ObjectKind::Literal),
        )
        .unwrap();
    store.consolidate_report().unwrap();
    for (id, kind, source) in [
        (entity, ObjectKind::Entity, "entity-source"),
        (literal, ObjectKind::Literal, "literal-source"),
    ] {
        let claim = store.get_claim(id).unwrap();
        assert_eq!(claim.status, aver_core::ClaimStatus::Active);
        assert_eq!(claim.object_kind, kind);
        assert_eq!(claim.source_refs, vec![source]);
    }
}

#[test]
fn migration_prevalidates_all_new_log_content_before_any_chunk() {
    for (untrusted_name, count) in [
        ("-----BEGIN PRIVATE KEY-----\nsynthetic-not-a-key", 257),
        ("/invented/.ssh/an invented sentence.", 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut legacy = String::new();
        for id in 1..=count {
            let record = serde_json::json!({"kind":"add_claim","ts":1,"claim_id":id,"subject":"sample","predicate":"concerns","object":format!("An invented sentence number {id}."),"source":"synthetic","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9,"provenance":"EXTRACTED","scope":"global"});
            legacy.push_str(&format!("{record}\n"));
        }
        std::fs::write(dir.path().join("log.jsonl"), &legacy).unwrap();
        replay(dir.path(), false).unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(
            store
                .add_claim(ClaimWrite::new(
                    "sample",
                    "concerns",
                    untrusted_name,
                    "synthetic"
                ))
                .is_err()
        );
        // Model an untrusted legacy projection row with no accepted log source.
        let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
        conn.execute("INSERT INTO entities(name,type_id,created_at,last_seen_at) SELECT ?1,id,1,1 FROM entity_types WHERE name='Thing'",[untrusted_name]).unwrap();
        assert!(store.migrate_literals().is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap(),
            legacy
        );
        assert_eq!(store.get_claim(1).unwrap().object_kind, ObjectKind::Entity);
        assert!(entity_exists(dir.path(), untrusted_name));
        assert!(store.preview_literal_migration().is_err());
    }
}
