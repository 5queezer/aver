//! T1 — opening a fresh Store applies the initial migration so the
//! `claims` table exists. See ADR-0006 for the schema.

use aver_core::Store;

#[test]
fn fresh_database_has_claims_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    assert!(
        store.has_table("claims"),
        "claims table should exist after open()"
    );
}

#[test]
fn fresh_database_has_entity_types_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("entity_types"),
        "entity_types table should exist after open()"
    );
}

#[test]
fn fresh_database_has_predicate_types_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("predicate_types"),
        "predicate_types table should exist after open()"
    );
}

#[test]
fn fresh_database_has_entity_type_closure_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("entity_type_closure"),
        "entity_type_closure table should exist after open()"
    );
}

#[test]
fn fresh_database_has_predicate_closure_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("predicate_closure"),
        "predicate_closure table should exist after open()"
    );
}

#[test]
fn fresh_database_has_contradictions_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("contradictions"),
        "contradictions table should exist after open()"
    );
}

#[test]
fn fresh_database_has_observations_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("observations"),
        "observations table should exist after open()"
    );
}

#[test]
fn fresh_database_has_vector_index_virtual_table() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();

    let create_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'vector_index'",
            [],
            |row| row.get(0),
        )
        .expect("vector_index virtual table should exist after open()");

    assert!(
        create_sql.contains("VIRTUAL TABLE") && create_sql.contains("vec0"),
        "vector_index should be a vec0 virtual table, got: {create_sql}"
    );
}

#[test]
fn fresh_database_has_ontology_enforcement_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();

    for trigger in [
        "claims_predicate_in_ontology_insert",
        "claims_predicate_in_ontology_update",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "{trigger} trigger should exist after open()");
    }
}

#[test]
fn claims_source_refs_must_be_valid_json_array() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid source_refs produced by Store should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO claims (subject, predicate, object, provenance, confidence, status,
                                 source_refs, agent_id, agent_kind, write_ts, created_at, last_seen_at)
             VALUES ('S', 'uses', 'O', 'USER_ASSERTED', 1.0, 'ACTIVE',
                     'not-json', 'local', 'HUMAN', 0, 0, 0)",
            [],
        )
        .expect_err("invalid JSON source_refs should be rejected");
    assert!(
        err.to_string()
            .contains("claims.source_refs must be a JSON array"),
        "unexpected error: {err}"
    );

    let err = conn
        .execute(
            "UPDATE claims SET source_refs = '{\"not\":\"an array\"}' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("non-array JSON source_refs should be rejected");
    assert!(
        err.to_string()
            .contains("claims.source_refs must be a JSON array"),
        "unexpected error: {err}"
    );
}
