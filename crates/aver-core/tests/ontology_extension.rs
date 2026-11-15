//! ADR-0031: a store-level `ontology.json` adds domain vocabulary to the core
//! ontology. Invented garden vocabulary only.

use std::path::Path;

use aver_core::{AgentKind, ClaimStatus, ClaimWrite, Error, Store, replay};
use rusqlite::Connection;

const GARDEN: &str = r#"{
  "version": 1,
  "predicates": [
    {"name": "grows_in", "parent": "relates_to"},
    {"name": "has_harvest", "parent": "concerns", "cardinality": "single"},
    {"name": "has_early_harvest", "parent": "has_harvest", "cardinality": "single"},
    {"name": "supersedes", "parent": "relates_to"}
  ],
  "entity_types": [{"name": "Plant", "parent": "Thing"}],
  "aliases": [{"alias": "planted_in", "predicate": "grows_in"}]
}"#;

fn llm(predicate: &str, object: &str) -> ClaimWrite {
    ClaimWrite::from_agent(
        "model",
        AgentKind::Llm,
        "plant:tomato",
        predicate,
        object,
        "synthetic",
    )
}

fn cardinality(dir: &Path, predicate: &str) -> String {
    Connection::open(dir.join("db.sqlite"))
        .unwrap()
        .query_row(
            "SELECT cardinality FROM predicate_types WHERE name = ?1",
            [predicate],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn absent_file_means_core_vocabulary_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.predicate_implies("about", "concerns").unwrap());
    assert!(store.predicate_implies("is_a", "is").unwrap());
    for core in [
        "concerns",
        "is",
        "triggered_by",
        "supersedes",
        "has_rationale",
    ] {
        store.add_claim(llm(core, "bed:north")).unwrap();
    }
    assert!(matches!(
        store.add_claim(llm("has_harvest", "july")),
        Err(Error::UnknownPredicate { .. })
    ));
}

#[test]
fn extension_adds_predicates_cardinality_aliases_and_types_on_open() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ontology.json"), GARDEN).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.predicate_implies("planted_in", "grows_in").unwrap());
    assert!(
        store
            .predicate_implies("has_early_harvest", "concerns")
            .unwrap()
    );
    assert!(store.entity_type_is_a("Plant", "Thing").unwrap());
    for (predicate, superseded) in [
        ("has_harvest", true),
        ("grows_in", false),
        ("planted_in", false),
    ] {
        let first = store.add_claim(llm(predicate, "first")).unwrap();
        let second = store.add_claim(llm(predicate, "second")).unwrap();
        store.consolidate_report().unwrap();
        let expected = if superseded {
            ClaimStatus::Superseded
        } else {
            ClaimStatus::Active
        };
        assert_eq!(
            store.get_claim(first).unwrap().status,
            expected,
            "{predicate}"
        );
        assert_eq!(store.get_claim(second).unwrap().status, ClaimStatus::Active);
    }
    store.close().unwrap();
    // Idempotent: reopening applies the same file without duplicates or errors.
    let store = Store::open(dir.path()).unwrap();
    assert!(store.predicate_implies("planted_in", "grows_in").unwrap());
    assert_eq!(cardinality(dir.path(), "has_harvest"), "single");
}

#[test]
fn replay_takes_the_declared_shape_with_the_file_and_never_rejects_without_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ontology.json"), GARDEN).unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store.add_claim(llm("has_harvest", "june")).unwrap();
    let second = store.add_claim(llm("has_harvest", "july")).unwrap();
    store.consolidate_report().unwrap();
    store.close().unwrap();
    let log = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    let statuses = |dir: &Path| {
        let store = Store::open(dir).unwrap();
        let statuses = [first, second].map(|id| store.get_claim(id).unwrap().status);
        store.close().unwrap();
        statuses
    };

    replay(dir.path(), true).unwrap();
    assert_eq!(cardinality(dir.path(), "has_harvest"), "single");
    assert_eq!(
        statuses(dir.path()),
        [ClaimStatus::Superseded, ClaimStatus::Active]
    );

    // The log stays the source of truth: without the file the same claims
    // replay, and the predicate comes back as an ordinary replay-created one.
    std::fs::remove_file(dir.path().join("ontology.json")).unwrap();
    replay(dir.path(), true).unwrap();
    assert_eq!(cardinality(dir.path(), "has_harvest"), "multi");
    assert_eq!(
        statuses(dir.path()),
        [ClaimStatus::Superseded, ClaimStatus::Active]
    );
    let store = Store::open(dir.path()).unwrap();
    assert!(!store.predicate_implies("has_harvest", "concerns").unwrap());
    store.close().unwrap();
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), log);
}

#[test]
fn bad_files_fail_closed_and_name_the_entry() {
    for (file, expected) in [
        (
            r#"{"version": 1, "predicate": []}"#,
            "unknown field `predicate`",
        ),
        (r#"{"version": 2}"#, "unsupported version 2"),
        ("{not json", "expected"),
        (
            r#"{"version": 1, "predicates": [{"name": "prefers", "parent": "concerns", "cardinality": "multi"}]}"#,
            r#"predicates[0] "prefers": is a core predicate"#,
        ),
        (
            r#"{"version": 1, "predicates": [{"name": "grows_in", "parent": "undeclared"}]}"#,
            r#"parent "undeclared" is neither"#,
        ),
        (
            r#"{"version": 1, "predicates": [{"name": "Grows In"}]}"#,
            "lowercase snake_case",
        ),
        (
            r#"{"version": 1, "predicates": [{"name": "x", "cardinality": "many"}]}"#,
            "unknown variant `many`",
        ),
        (
            r#"{"version": 1, "entity_types": [{"name": "Decision", "parent": "Thing"}]}"#,
            "is a core entity type",
        ),
        (
            r#"{"version": 1, "predicates": [{"name": "grows_in"}], "aliases": [{"alias": "about", "predicate": "grows_in"}]}"#,
            r#"aliases[0] "about": already names another predicate or alias"#,
        ),
        (
            r#"{"version": 1, "aliases": [{"alias": "planted_in", "predicate": "grows_in"}]}"#,
            r#"predicate "grows_in" is neither"#,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ontology.json"), file).unwrap();
        let error = Store::open(dir.path()).err().unwrap();
        let message = error.to_string();
        assert!(matches!(error, Error::OntologyFile { .. }), "{message}");
        assert!(
            message.contains("ontology.json") && message.contains(expected),
            "{message}"
        );
    }
}

#[test]
fn rows_an_older_seed_added_stay_usable_without_the_file() {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap().close().unwrap();
    // As a store seeded by an older binary that shipped this predicate.
    Connection::open(dir.path().join("db.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO predicate_types (name, parent_id, cardinality)
             SELECT 'has_harvest', id, 'single' FROM predicate_types WHERE name = 'concerns'",
            [],
        )
        .unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.predicate_implies("has_harvest", "concerns").unwrap());
    let first = store.add_claim(llm("has_harvest", "june")).unwrap();
    store.add_claim(llm("has_harvest", "july")).unwrap();
    store.consolidate_report().unwrap();
    assert_eq!(
        store.get_claim(first).unwrap().status,
        ClaimStatus::Superseded
    );
    assert_eq!(cardinality(dir.path(), "has_harvest"), "single");
}
