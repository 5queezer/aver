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
fn entity_type_name_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute("INSERT INTO entity_types (name) VALUES ('   ')", [])
        .expect_err("blank entity type names should be rejected");
    assert!(
        err.to_string()
            .contains("entity_types.name must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn entity_type_parent_must_not_self_reference() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO entity_types (id, name, parent_id) VALUES (9999, 'SelfType', 9999)",
            [],
        )
        .expect_err("entity types should not be their own parent");
    assert!(
        err.to_string()
            .contains("entity_types.parent_id must differ from id"),
        "unexpected error: {err}"
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
fn predicate_type_name_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute("INSERT INTO predicate_types (name) VALUES ('   ')", [])
        .expect_err("blank predicate type names should be rejected");
    assert!(
        err.to_string()
            .contains("predicate_types.name must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn predicate_type_parent_must_not_self_reference() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO predicate_types (id, name, parent_id) VALUES (9999, 'self_predicate', 9999)",
            [],
        )
        .expect_err("predicate types should not be their own parent");
    assert!(
        err.to_string()
            .contains("predicate_types.parent_id must differ from id"),
        "unexpected error: {err}"
    );
}

#[test]
fn predicate_alias_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let predicate_id: i64 = conn
        .query_row(
            "SELECT id FROM predicate_types WHERE name = 'uses'",
            [],
            |row| row.get(0),
        )
        .expect("seeded predicate should exist");
    let err = conn
        .execute(
            "INSERT INTO predicate_alias (alias, predicate_id, created_at, note)
             VALUES ('   ', ?1, 1, 'test')",
            [predicate_id],
        )
        .expect_err("blank predicate aliases should be rejected");
    assert!(
        err.to_string()
            .contains("predicate_alias.alias must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn predicate_alias_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let predicate_id: i64 = conn
        .query_row(
            "SELECT id FROM predicate_types WHERE name = 'uses'",
            [],
            |row| row.get(0),
        )
        .expect("seeded predicate should exist");
    let err = conn
        .execute(
            "INSERT INTO predicate_alias (alias, predicate_id, created_at, note)
             VALUES ('uses_test_alias', ?1, 0, 'test')",
            [predicate_id],
        )
        .expect_err("predicate alias timestamps should be positive");
    assert!(
        err.to_string()
            .contains("predicate_alias.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_predicate_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('   ', 'relates_to', 'agent', 1)",
            [],
        )
        .expect_err("blank ontology extension predicates should be rejected");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.predicate must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_parent_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('uses', '   ', 'agent', 1)",
            [],
        )
        .expect_err("blank ontology extension parents should be rejected");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.parent must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_agent_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('uses', 'relates_to', '   ', 1)",
            [],
        )
        .expect_err("blank ontology extension agent ids should be rejected");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.agent_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_agent_id_allows_only_portable_identifier_chars() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
         VALUES ('uses', 'relates_to', 'agent_1-ok', 1)",
        [],
    )
    .expect("portable ontology extension agent ids should remain valid");

    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('has', 'relates_to', 'bad id!', 1)",
            [],
        )
        .expect_err("ontology extension agent ids with spaces/punctuation should be rejected");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.agent_id contains invalid characters"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('uses', 'relates_to', 'agent', 0)",
            [],
        )
        .expect_err("ontology extension timestamps should be positive");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_parent_must_exist() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('uses', 'missing_parent', 'agent', 1)",
            [],
        )
        .expect_err("ontology extension parents should exist in predicate_types");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.parent must exist in predicate_types"),
        "unexpected error: {err}"
    );
}

#[test]
fn ontology_extension_predicate_must_exist() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
             VALUES ('missing_predicate', 'relates_to', 'agent', 1)",
            [],
        )
        .expect_err("ontology extension predicates should exist in predicate_types");
    assert!(
        err.to_string()
            .contains("ontology_extension_log.predicate must exist in predicate_types"),
        "unexpected error: {err}"
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
fn entity_name_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let type_id: i64 = conn
        .query_row(
            "SELECT id FROM entity_types WHERE name = 'Thing'",
            [],
            |row| row.get(0),
        )
        .expect("seeded Thing entity type should exist");
    let err = conn
        .execute(
            "INSERT INTO entities (name, type_id, created_at, last_seen_at)
             VALUES ('   ', ?1, 1, 1)",
            [type_id],
        )
        .expect_err("blank entity names should be rejected");
    assert!(
        err.to_string().contains("entities.name must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn entity_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let type_id: i64 = conn
        .query_row(
            "SELECT id FROM entity_types WHERE name = 'Thing'",
            [],
            |row| row.get(0),
        )
        .expect("seeded Thing entity type should exist");
    let err = conn
        .execute(
            "INSERT INTO entities (name, type_id, created_at, last_seen_at)
             VALUES ('Aver', ?1, 0, 1)",
            [type_id],
        )
        .expect_err("entity created_at should be positive");
    assert!(
        err.to_string()
            .contains("entities.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn entity_last_seen_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let type_id: i64 = conn
        .query_row(
            "SELECT id FROM entity_types WHERE name = 'Thing'",
            [],
            |row| row.get(0),
        )
        .expect("seeded Thing entity type should exist");
    let err = conn
        .execute(
            "INSERT INTO entities (name, type_id, created_at, last_seen_at)
             VALUES ('Aver', ?1, 1, 0)",
            [type_id],
        )
        .expect_err("entity last_seen_at should be positive");
    assert!(
        err.to_string()
            .contains("entities.last_seen_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn entity_last_seen_at_must_not_precede_created_at() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let type_id: i64 = conn
        .query_row(
            "SELECT id FROM entity_types WHERE name = 'Thing'",
            [],
            |row| row.get(0),
        )
        .expect("seeded Thing entity type should exist");
    let err = conn
        .execute(
            "INSERT INTO entities (name, type_id, created_at, last_seen_at)
             VALUES ('Aver', ?1, 10, 5)",
            [type_id],
        )
        .expect_err("entity last_seen_at should not precede created_at");
    assert!(
        err.to_string()
            .contains("entities.last_seen_at must be >= created_at"),
        "unexpected error: {err}"
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
fn privacy_rejection_reason_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO privacy_rejections (reason, count) VALUES ('   ', 1)",
            [],
        )
        .expect_err("blank privacy rejection reasons should be rejected");
    assert!(
        err.to_string()
            .contains("privacy_rejections.reason must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn privacy_rejection_count_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO privacy_rejections (reason, count) VALUES ('entropy', 0)",
            [],
        )
        .expect_err("privacy rejection counts should be positive");
    assert!(
        err.to_string()
            .contains("privacy_rejections.count must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn contradiction_reason_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO contradictions (claim_id, reason, created_at)
             VALUES (?1, '   ', 1)",
            [claim_id],
        )
        .expect_err("blank contradiction reasons should be rejected");
    assert!(
        err.to_string()
            .contains("contradictions.reason must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn contradiction_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO contradictions (claim_id, reason, created_at)
             VALUES (?1, 'unsupported by source', 0)",
            [claim_id],
        )
        .expect_err("contradiction timestamps must be positive");
    assert!(
        err.to_string()
            .contains("contradictions.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn contradiction_new_claim_must_not_equal_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO contradictions (claim_id, reason, new_claim_id, created_at)
             VALUES (?1, 'self-contradiction is not useful evidence', ?1, 1)",
            [claim_id],
        )
        .expect_err("contradictions should not point new_claim_id at the same claim");
    assert!(
        err.to_string()
            .contains("contradictions.new_claim_id must differ from claim_id"),
        "unexpected error: {err}"
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
fn episodic_event_session_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('   ', 'message', 'payload', 'test', 'local', 'HUMAN', 1)",
            [],
        )
        .expect_err("blank episodic event session_id should be rejected");
    assert!(
        err.to_string()
            .contains("episodic_events.session_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn episodic_event_kind_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('session-1', '   ', 'payload', 'test', 'local', 'HUMAN', 1)",
            [],
        )
        .expect_err("blank episodic event kind should be rejected");
    assert!(
        err.to_string()
            .contains("episodic_events.kind must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn episodic_event_source_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('session-1', 'message', 'payload', '   ', 'local', 'HUMAN', 1)",
            [],
        )
        .expect_err("blank episodic event source should be rejected");
    assert!(
        err.to_string()
            .contains("episodic_events.source must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn episodic_event_agent_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('session-1', 'message', 'payload', 'test', '   ', 'HUMAN', 1)",
            [],
        )
        .expect_err("blank episodic event agent_id should be rejected");
    assert!(
        err.to_string()
            .contains("episodic_events.agent_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn episodic_event_agent_id_allows_only_portable_identifier_chars() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
         VALUES ('session-1', 'message', 'payload', 'test', 'agent_1-ok', 'HUMAN', 1)",
        [],
    )
    .expect("portable agent ids should remain valid");

    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('session-1', 'message', 'payload', 'test', 'bad id!', 'HUMAN', 1)",
            [],
        )
        .expect_err("agent ids with spaces/punctuation should be rejected");
    assert!(
        err.to_string()
            .contains("episodic_events.agent_id contains invalid characters"),
        "unexpected error: {err}"
    );
}

#[test]
fn episodic_event_timestamp_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO episodic_events (session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES ('session-1', 'message', 'payload', 'test', 'local', 'HUMAN', 0)",
            [],
        )
        .expect_err("episodic event timestamps must be positive");
    assert!(
        err.to_string()
            .contains("episodic_events.ts must be positive"),
        "unexpected error: {err}"
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
                     'not-json', 'local', 'HUMAN', 1, 1, 1)",
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

#[test]
fn claims_source_refs_array_elements_must_be_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid source_refs produced by Store should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "UPDATE claims SET source_refs = '[\"manual\", \"event:1\"]' WHERE id = ?1",
        [valid_id],
    )
    .expect("string source_refs should remain valid");

    let err = conn
        .execute(
            "UPDATE claims SET source_refs = '[\"manual\", 42]' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("non-text source_refs elements should be rejected");
    assert!(
        err.to_string()
            .contains("claims.source_refs elements must be text"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_source_refs_must_not_be_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid source_refs produced by Store should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE claims SET source_refs = '[]' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("empty source_refs should be rejected");
    assert!(
        err.to_string()
            .contains("claims.source_refs must not be empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_source_refs_text_elements_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid source_refs produced by Store should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE claims SET source_refs = '[\"test\", \"   \"]' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("blank source_refs elements should be rejected");
    assert!(
        err.to_string()
            .contains("claims.source_refs elements must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_agent_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE claims SET agent_id = '   ' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("blank claim agent_id should be rejected");
    assert!(
        err.to_string()
            .contains("claims.agent_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_subject_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE claims SET subject = '   ' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("blank claim subject should be rejected");
    assert!(
        err.to_string().contains("claims.subject must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_object_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute("UPDATE claims SET object = '   ' WHERE id = ?1", [valid_id])
        .expect_err("blank claim object should be rejected");
    assert!(
        err.to_string().contains("claims.object must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute("UPDATE claims SET created_at = 0 WHERE id = ?1", [valid_id])
        .expect_err("non-positive claim created_at should be rejected");
    assert!(
        err.to_string()
            .contains("claims.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_last_seen_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE claims SET last_seen_at = 0 WHERE id = ?1",
            [valid_id],
        )
        .expect_err("non-positive claim last_seen_at should be rejected");
    assert!(
        err.to_string()
            .contains("claims.last_seen_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_last_seen_at_must_not_precede_created_at() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "UPDATE claims SET created_at = 10, last_seen_at = 10 WHERE id = ?1",
        [valid_id],
    )
    .expect("matching positive timestamps should remain valid");

    let err = conn
        .execute(
            "UPDATE claims SET created_at = 10, last_seen_at = 9 WHERE id = ?1",
            [valid_id],
        )
        .expect_err("claim last_seen_at before created_at should be rejected");
    assert!(
        err.to_string()
            .contains("claims.last_seen_at must be >= created_at"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_write_ts_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute("UPDATE claims SET write_ts = 0 WHERE id = ?1", [valid_id])
        .expect_err("non-positive claim write_ts should be rejected");
    assert!(
        err.to_string().contains("claims.write_ts must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn claims_agent_id_allows_only_portable_identifier_chars() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let valid_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("valid claim should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "UPDATE claims SET agent_id = 'agent_1-ok' WHERE id = ?1",
        [valid_id],
    )
    .expect("portable claim agent ids should remain valid");

    let err = conn
        .execute(
            "UPDATE claims SET agent_id = 'bad id!' WHERE id = ?1",
            [valid_id],
        )
        .expect_err("claim agent ids with spaces/punctuation should be rejected");
    assert!(
        err.to_string()
            .contains("claims.agent_id contains invalid characters"),
        "unexpected error: {err}"
    );
}

#[test]
fn candidate_claim_subject_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, '   ', 'uses', 'SQLite', 1)",
            [event_id],
        )
        .expect_err("blank candidate claim subject should be rejected");
    assert!(
        err.to_string()
            .contains("candidate_claims.subject must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn candidate_claim_predicate_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, 'Aver', '   ', 'SQLite', 1)",
            [event_id],
        )
        .expect_err("blank candidate claim predicate should be rejected");
    assert!(
        err.to_string()
            .contains("candidate_claims.predicate must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn candidate_claim_object_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, 'Aver', 'uses', '   ', 1)",
            [event_id],
        )
        .expect_err("blank candidate claim object should be rejected");
    assert!(
        err.to_string()
            .contains("candidate_claims.object must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejected_candidate_claims_must_have_nonblank_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims SET status = 'REJECTED', rejection_reason = '   ' WHERE id = ?1",
            [candidate_id],
        )
        .expect_err("rejected candidate claims need a nonblank reason");
    assert!(
        err.to_string().contains(
            "candidate_claims.rejection_reason must not be blank when status is REJECTED"
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn promoted_candidate_claims_must_reference_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims SET status = 'PROMOTED', promoted_claim_id = NULL WHERE id = ?1",
            [candidate_id],
        )
        .expect_err("promoted candidate claims need a promoted claim id");
    assert!(
        err.to_string()
            .contains("candidate_claims.promoted_claim_id must be set when status is PROMOTED"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejected_candidate_claims_must_not_reference_promoted_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims
             SET status = 'REJECTED', rejection_reason = 'unsupported', promoted_claim_id = ?1
             WHERE id = ?2",
            [claim_id, candidate_id],
        )
        .expect_err("rejected candidate claims must not point at promoted claims");
    assert!(
        err.to_string()
            .contains("candidate_claims.promoted_claim_id must be NULL when status is REJECTED"),
        "unexpected error: {err}"
    );
}

#[test]
fn promoted_candidate_claims_must_not_have_rejection_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims
             SET status = 'PROMOTED', promoted_claim_id = ?1, rejection_reason = 'unsupported'
             WHERE id = ?2",
            [claim_id, candidate_id],
        )
        .expect_err("promoted candidate claims must not keep rejection reasons");
    assert!(
        err.to_string()
            .contains("candidate_claims.rejection_reason must be NULL when status is PROMOTED"),
        "unexpected error: {err}"
    );
}

#[test]
fn pending_candidate_claims_must_not_have_rejection_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims SET rejection_reason = 'unsupported' WHERE id = ?1",
            [candidate_id],
        )
        .expect_err("pending candidate claims must not keep rejection reasons");
    assert!(
        err.to_string()
            .contains("candidate_claims.rejection_reason must be NULL when status is PENDING"),
        "unexpected error: {err}"
    );
}

#[test]
fn pending_candidate_claims_must_not_reference_promoted_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "SQLite")
        .expect("candidate insert should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "UPDATE candidate_claims SET promoted_claim_id = ?1 WHERE id = ?2",
            [claim_id, candidate_id],
        )
        .expect_err("pending candidate claims must not point at promoted claims");
    assert!(
        err.to_string()
            .contains("candidate_claims.promoted_claim_id must be NULL when status is PENDING"),
        "unexpected error: {err}"
    );
}

#[test]
fn candidate_claim_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "message", "payload", "test")
        .expect("event insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, 'Aver', 'uses', 'SQLite', 0)",
            [event_id],
        )
        .expect_err("candidate claim timestamps must be positive");
    assert!(
        err.to_string()
            .contains("candidate_claims.created_at must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_source_event_ids_must_be_valid_json_array() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let event_id = store
        .record_event("session-1", "user_message", "safe payload", "test")
        .expect("event insert should succeed");
    let observation_id = store
        .record_observation(
            "session-1",
            "Aver remembers schema facts.",
            aver_core::ObservationRelevance::High,
            &[event_id],
            "observer",
        )
        .expect("valid source_event_ids produced by Store should insert");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('bad-json', 'session-1', 'bad', 'high', 'not-json',
                     'observer', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("invalid JSON source_event_ids should be rejected");
    assert!(
        err.to_string()
            .contains("observations.source_event_ids must be a JSON array"),
        "unexpected error: {err}"
    );

    let err = conn
        .execute(
            "UPDATE observations SET source_event_ids = '{\"not\":\"an array\"}' WHERE id = ?1",
            [observation_id],
        )
        .expect_err("non-array JSON source_event_ids should be rejected");
    assert!(
        err.to_string()
            .contains("observations.source_event_ids must be a JSON array"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_agent_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('blank-agent', 'session-1', 'bad', 'high', '[1]',
                     '   ', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("blank observation agent_id should be rejected");
    assert!(
        err.to_string()
            .contains("observations.agent_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_agent_id_allows_only_portable_identifier_chars() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                   agent_id, agent_kind, derivation, ts)
         VALUES ('valid-agent', 'session-1', 'ok', 'high', '[1]',
                 'agent_1-ok', 'LLM', 'test', 1)",
        [],
    )
    .expect("portable observation agent ids should remain valid");

    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('bad-agent', 'session-1', 'bad', 'high', '[1]',
                     'bad id!', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("observation agent ids with spaces/punctuation should be rejected");
    assert!(
        err.to_string()
            .contains("observations.agent_id contains invalid characters"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_session_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('blank-session', '   ', 'bad', 'high', '[1]',
                     'agent', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("blank observation session_id should be rejected");
    assert!(
        err.to_string()
            .contains("observations.session_id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_content_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('blank-content', 'session-1', '   ', 'high', '[1]',
                     'agent', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("blank observation content should be rejected");
    assert!(
        err.to_string()
            .contains("observations.content must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_derivation_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('blank-derivation', 'session-1', 'content', 'high', '[1]',
                     'agent', 'LLM', '   ', 1)",
            [],
        )
        .expect_err("blank observation derivation should be rejected");
    assert!(
        err.to_string()
            .contains("observations.derivation must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_id_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('   ', 'session-1', 'content', 'high', '[1]',
                     'agent', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("blank observation id should be rejected");
    assert!(
        err.to_string()
            .contains("observations.id must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_timestamp_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('zero-ts', 'session-1', 'content', 'high', '[1]',
                     'agent', 'LLM', 'test', 0)",
            [],
        )
        .expect_err("observation timestamps must be positive");
    assert!(
        err.to_string().contains("observations.ts must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_source_event_ids_array_elements_must_be_integers() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                   agent_id, agent_kind, derivation, ts)
         VALUES ('ok-ids', 'session-1', 'ok', 'high', '[1, 2, 3]',
                 'observer', 'LLM', 'test', 1)",
        [],
    )
    .expect("integer source_event_ids should remain valid");

    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('bad-id', 'session-1', 'bad', 'high', '[1, \"not-an-id\"]',
                     'observer', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("non-integer source_event_ids elements should be rejected");
    assert!(
        err.to_string()
            .contains("observations.source_event_ids elements must be integers"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_source_event_ids_must_not_be_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('empty-ids', 'session-1', 'bad', 'high', '[]',
                     'observer', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("empty source_event_ids should be rejected");
    assert!(
        err.to_string()
            .contains("observations.source_event_ids must not be empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn observations_source_event_ids_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO observations (id, session_id, content, relevance, source_event_ids,
                                       agent_id, agent_kind, derivation, ts)
             VALUES ('zero-id', 'session-1', 'bad', 'high', '[0]',
                     'observer', 'LLM', 'test', 1)",
            [],
        )
        .expect_err("non-positive source_event_ids should be rejected");
    assert!(
        err.to_string()
            .contains("observations.source_event_ids elements must be positive"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_embedding_json_must_be_null_or_json_array() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    store
        .add_vector_chunk(claim_id, "chunk without embedding", "nomic-embed-text")
        .expect("NULL embedding_json should remain valid");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 'bad', 'nomic-embed-text', 'not-json', 1)",
            [claim_id],
        )
        .expect_err("invalid JSON embedding_json should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.embedding_json must be NULL or a JSON array"),
        "unexpected error: {err}"
    );

    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 'bad', 'nomic-embed-text', '{\"not\":\"an array\"}', 1)",
            [claim_id],
        )
        .expect_err("non-array JSON embedding_json should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.embedding_json must be NULL or a JSON array"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_embedding_json_array_elements_must_be_numeric() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
         VALUES (?1, 'ok', 'nomic-embed-text', '[0.1, 2, -3.5]', 1)",
        [claim_id],
    )
    .expect("numeric JSON array embedding_json should remain valid");

    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 'bad', 'nomic-embed-text', '[0.1, \"not-a-number\"]', 1)",
            [claim_id],
        )
        .expect_err("non-numeric embedding_json elements should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.embedding_json elements must be numeric"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_embedding_json_must_not_be_empty_array() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 'bad', 'nomic-embed-text', '[]', 1)",
            [claim_id],
        )
        .expect_err("empty embedding_json arrays should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.embedding_json must not be empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_text_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, '   ', 'nomic-embed-text', 1)",
            [claim_id],
        )
        .expect_err("blank vector chunk text should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.text must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_embedding_model_must_not_be_blank() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, 'valid chunk', '   ', 1)",
            [claim_id],
        )
        .expect_err("blank embedding_model should be rejected");
    assert!(
        err.to_string()
            .contains("vector_chunks.embedding_model must not be blank"),
        "unexpected error: {err}"
    );
}

#[test]
fn vector_chunk_created_at_must_be_positive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");
    let claim_id = store
        .add_claim("Aver", "uses", "SQLite", "test")
        .expect("claim insert should succeed");
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn
        .execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, 'valid chunk', 'nomic-embed-text', 0)",
            [claim_id],
        )
        .expect_err("vector chunk timestamps must be positive");
    assert!(
        err.to_string()
            .contains("vector_chunks.created_at must be positive"),
        "unexpected error: {err}"
    );
}
