//! Migration 0010: predicate cardinality. Consolidation supersedes an older
//! object only for single-valued predicates; multi-valued predicates such
//! as `defines` and `calls` keep every object.

use aver_core::{ClaimStatus, ClaimWrite, Store};
use rusqlite::Connection;

fn statuses(store: &Store, ids: &[i64]) -> Vec<ClaimStatus> {
    ids.iter()
        .map(|id| store.get_claim(*id).unwrap().status)
        .collect()
}

#[test]
fn multi_valued_predicates_keep_every_object_through_consolidation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let defines_a = store
        .add_claim(ClaimWrite::new("File:x", "defines", "Function:a", "scan"))
        .unwrap();
    let defines_b = store
        .add_claim(ClaimWrite::new("File:x", "defines", "Function:b", "scan"))
        .unwrap();
    let calls_a = store
        .add_claim(ClaimWrite::new(
            "Function:main",
            "calls",
            "Function:a",
            "scan",
        ))
        .unwrap();
    let calls_b = store
        .add_claim(ClaimWrite::new(
            "Function:main",
            "calls",
            "Function:b",
            "scan",
        ))
        .unwrap();

    let report = store.consolidate_report().unwrap();
    assert_eq!(
        report.superseded, 0,
        "multi-valued predicates never conflict"
    );
    assert_eq!(
        statuses(&store, &[defines_a, defines_b, calls_a, calls_b]),
        vec![ClaimStatus::Active; 4]
    );
}

#[test]
fn single_valued_predicates_are_superseded_by_a_newer_object() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let tabs = store
        .add_claim(ClaimWrite::new("User", "prefers", "tabs", "chat"))
        .unwrap();
    let spaces = store
        .add_claim(ClaimWrite::new("User", "prefers", "spaces", "chat"))
        .unwrap();
    // An alias of a single-valued predicate follows it.
    let old_name = store
        .add_claim(ClaimWrite::new("aver-cli", "binary-name", "aver0", "notes"))
        .unwrap();
    let new_name = store
        .add_claim(ClaimWrite::new("aver-cli", "binary-name", "aver", "notes"))
        .unwrap();

    let report = store.consolidate_report().unwrap();
    assert_eq!(report.superseded, 2);
    assert_eq!(
        statuses(&store, &[tabs, spaces, old_name, new_name]),
        vec![
            ClaimStatus::Superseded,
            ClaimStatus::Active,
            ClaimStatus::Superseded,
            ClaimStatus::Active,
        ]
    );
}

#[test]
fn ontology_extensions_default_to_multi_valued() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // USER_ASSERTED writes auto-extend the ontology (ADR-0018); the new
    // predicate must not inherit single-valued semantics.
    let first = store
        .add_claim(ClaimWrite::new(
            "Service:api",
            "brand_new_rel",
            "a",
            "notes",
        ))
        .unwrap();
    let second = store
        .add_claim(ClaimWrite::new(
            "Service:api",
            "brand_new_rel",
            "b",
            "notes",
        ))
        .unwrap();
    assert_eq!(store.consolidate_report().unwrap().superseded, 0);
    assert_eq!(
        statuses(&store, &[first, second]),
        vec![ClaimStatus::Active; 2]
    );

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let cardinality: String = conn
        .query_row(
            "SELECT cardinality FROM predicate_types WHERE name = 'brand_new_rel'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cardinality, "multi");
    let singles: Vec<String> = conn
        .prepare("SELECT name FROM predicate_types WHERE cardinality = 'single' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        singles,
        [
            "binary_name",
            "impl_stays_in",
            "owned_by",
            "prefers",
            "splits_by"
        ]
    );
}

#[test]
fn existing_store_gains_cardinality_on_open() {
    // A store at the pre-0010 schema gets the column and the seed marks on
    // the next open (ADR-0019 §6 versioned migrations).
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store
            .add_claim(ClaimWrite::new("User", "prefers", "tabs", "chat"))
            .unwrap();
        store.close().unwrap();
    }
    {
        let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
        // Simulate the pre-0010 schema: drop the column and step the version back.
        conn.execute_batch("ALTER TABLE predicate_types DROP COLUMN cardinality")
            .unwrap();
        conn.pragma_update(None, "user_version", 119).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim(ClaimWrite::new("User", "prefers", "spaces", "chat"))
        .unwrap();
    assert_eq!(store.consolidate_report().unwrap().superseded, 1);
}
