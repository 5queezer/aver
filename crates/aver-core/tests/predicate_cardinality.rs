//! Migration 0010: predicate cardinality. Consolidation supersedes an older
//! object only for single-valued predicates; multi-valued predicates such
//! as `defines` and `calls` keep every object.

use aver_core::{ClaimStatus, ClaimWrite, Store, replay};
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

fn assert_newest_single_valued_claim_survives(values: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let ids = values
        .iter()
        .map(|value| {
            store
                .add_claim(ClaimWrite::new("User", "prefers", *value, "chat"))
                .unwrap()
        })
        .collect::<Vec<_>>();

    let report = store.consolidate_report().unwrap();
    assert_eq!(report.superseded, values.len() - 1);
    let expected = ids
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 == ids.len() {
                ClaimStatus::Active
            } else {
                ClaimStatus::Superseded
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(statuses(&store, &ids), expected);

    let second = store.consolidate_report().unwrap();
    assert_eq!(second.superseded, 0);
    assert_eq!(statuses(&store, &ids), expected);
    store.close().unwrap();

    std::fs::remove_file(dir.path().join("db.sqlite")).unwrap();
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-wal"));
    let _ = std::fs::remove_file(dir.path().join("db.sqlite-shm"));
    replay(dir.path(), false).unwrap();
    let replayed = Store::open(dir.path()).unwrap();
    assert_eq!(statuses(&replayed, &ids), expected);
}

#[test]
fn single_valued_reversion_keeps_the_newest_write_active() {
    assert_newest_single_valued_claim_survives(&["a", "b", "a"]);
}

#[test]
fn single_valued_duplicates_before_a_change_still_leave_the_new_value() {
    assert_newest_single_valued_claim_survives(&["a", "a", "b"]);
}

#[test]
fn single_valued_trailing_duplicates_keep_the_newest_duplicate() {
    assert_newest_single_valued_claim_survives(&["b", "a", "a"]);
}

#[test]
fn single_valued_multiple_reversions_keep_the_newest_write() {
    assert_newest_single_valued_claim_survives(&["a", "b", "c", "b", "a"]);
}

#[test]
fn single_valued_retirement_does_not_revive_an_older_value() {
    let direct_dir = tempfile::tempdir().unwrap();
    let direct = Store::open(direct_dir.path()).unwrap();
    let direct_a = direct
        .add_claim(ClaimWrite::new("User", "prefers", "a", "chat"))
        .unwrap();
    let direct_b = direct
        .add_claim(ClaimWrite::new("User", "prefers", "b", "chat"))
        .unwrap();
    direct.retire_claim(direct_b, "explicitly invalid").unwrap();
    direct.consolidate_report().unwrap();
    assert_eq!(
        statuses(&direct, &[direct_a, direct_b]),
        vec![ClaimStatus::Superseded, ClaimStatus::Invalidated]
    );

    let consolidated_dir = tempfile::tempdir().unwrap();
    let consolidated = Store::open(consolidated_dir.path()).unwrap();
    let consolidated_a = consolidated
        .add_claim(ClaimWrite::new("User", "prefers", "a", "chat"))
        .unwrap();
    let consolidated_b = consolidated
        .add_claim(ClaimWrite::new("User", "prefers", "b", "chat"))
        .unwrap();
    consolidated.consolidate_report().unwrap();
    consolidated
        .retire_claim(consolidated_b, "explicitly invalid")
        .unwrap();
    consolidated.consolidate_report().unwrap();
    assert_eq!(
        statuses(&consolidated, &[consolidated_a, consolidated_b]),
        statuses(&direct, &[direct_a, direct_b])
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
            "closed_at",
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
        // Simulate the complete pre-0010 schema, including removal of later
        // object typing/event-link columns before stepping the version back.
        conn.execute_batch(
            "DROP INDEX claims_scope_context;
            DROP TRIGGER claims_validity_window_insert;
            DROP TRIGGER claims_validity_window_update;
            DROP TABLE claim_verifications;
            ALTER TABLE claims DROP COLUMN valid_until;
            ALTER TABLE claims DROP COLUMN valid_from;
            ALTER TABLE claims DROP COLUMN context;
            DROP INDEX claims_event_id;
            ALTER TABLE claims DROP COLUMN event_id;
            ALTER TABLE claims DROP COLUMN object_kind;
            ALTER TABLE predicate_types DROP COLUMN cardinality",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 119).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim(ClaimWrite::new("User", "prefers", "spaces", "chat"))
        .unwrap();
    assert_eq!(store.consolidate_report().unwrap().superseded, 1);
}

#[test]
fn seed_includes_the_extractor_edges_as_multi_valued_predicates() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT child.name, parent.name, child.cardinality
               FROM predicate_types child
               JOIN predicate_types parent ON parent.id = child.parent_id
              WHERE child.name IN ('extends', 'implements', 'has_test')
              ORDER BY child.name",
        )
        .unwrap();
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            (
                "extends".to_string(),
                "depends_on".to_string(),
                "multi".to_string()
            ),
            (
                "has_test".to_string(),
                "relates_to".to_string(),
                "multi".to_string()
            ),
            (
                "implements".to_string(),
                "depends_on".to_string(),
                "multi".to_string()
            ),
        ]
    );
}
