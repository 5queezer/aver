//! ADR-0018 ontology enforcement on claim writes.
//!
//! Three integration tests, one per failure axis, plus a defense-in-depth
//! test for the BEFORE INSERT trigger.

use aver_core::{AgentKind, Error, Store};
use rusqlite::Connection;

#[test]
fn unknown_predicate_rejected_for_extracted() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .add_claim_from_agent(
            "parser_agent",
            AgentKind::DeterministicParser,
            "auth_service",
            "orchestrates",
            "session_token",
            "tree_sitter",
        )
        .expect_err("EXTRACTED writes with unknown predicate must reject");

    assert!(
        matches!(err, Error::UnknownPredicate { ref name, .. } if name == "orchestrates"),
        "expected UnknownPredicate for 'orchestrates', got {err:?}"
    );

    // No claim row written.
    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM claims WHERE predicate = 'orchestrates'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);

    // No JSONL log line written either (matches privacy-rejection contract).
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap_or_default();
    assert!(
        !log.contains("orchestrates"),
        "rejected predicate must not appear in log.jsonl"
    );
}

#[test]
fn unknown_predicate_diagnostic_includes_available_predicates_and_suggestion() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .add_claim_from_agent(
            "llm_agent",
            AgentKind::Llm,
            "app_server",
            "depends_onn",
            "database",
            "llm",
        )
        .expect_err("LLM writes with unknown predicates should explain the vocabulary");

    let Error::UnknownPredicate { name } = err else {
        panic!("expected UnknownPredicate, got {err:?}");
    };

    assert_eq!(name, "depends_onn");

    let msg = store.describe_unknown_predicate(&name).unwrap();
    assert!(msg.contains("did you mean `depends_on`?"), "{msg}");
    assert!(msg.contains("available predicates:"), "{msg}");
    assert!(msg.contains("`depends_on`"), "{msg}");
    assert!(msg.contains("`uses`"), "{msg}");
    assert!(msg.contains("accepted aliases:"), "{msg}");
    assert!(msg.contains("`has-module`"), "{msg}");
}

#[test]
fn requires_alias_resolves_to_depends_on_for_claims_and_graph_filters() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim_from_agent(
            "llm_agent",
            AgentKind::Llm,
            "app_server",
            "requires",
            "database",
            "llm",
        )
        .expect("requires should be accepted as an alias for depends_on");

    let claim = store.get_claim(claim_id).unwrap();
    assert_eq!(claim.predicate, "requires");
    assert!(store.predicate_implies("requires", "depends_on").unwrap());
    assert!(store.predicate_implies("requires", "relates_to").unwrap());

    let expansion = store
        .expand("app_server", 1, Some(&["depends_on"]))
        .expect("depends_on filter should include requires alias edges");
    assert!(
        expansion
            .edges
            .iter()
            .any(|edge| edge.id == claim_id && edge.predicate == "requires"),
        "depends_on graph filter should include the requires alias edge: {expansion:?}"
    );
}

#[test]
fn unknown_predicate_diagnostic_uses_fuzzy_alias_aware_suggestions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .add_claim_from_agent(
            "llm_agent",
            AgentKind::Llm,
            "app_server",
            "requirse",
            "database",
            "llm",
        )
        .expect_err("misspelled aliases should still reject");

    let Error::UnknownPredicate { name } = err else {
        panic!("expected UnknownPredicate, got {err:?}");
    };
    assert_eq!(name, "requirse");

    let msg = store.describe_unknown_predicate(&name).unwrap();
    assert!(
        msg.contains("did you mean `requires` (alias for `depends_on`)?"),
        "{msg}"
    );
    assert!(msg.contains("available predicates:"), "{msg}");
    assert!(msg.contains("`depends_on`"), "{msg}");
    assert!(msg.contains("accepted aliases:"), "{msg}");
    assert!(msg.contains("`requires`"), "{msg}");
}

#[test]
fn unknown_predicate_auto_added_for_user_asserted() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("auth", "orchestrates_v2", "session_token", "manual")
        .expect("USER_ASSERTED auto-extends the ontology");
    assert!(claim_id > 0);

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let (created_via, parent_name): (String, String) = conn
        .query_row(
            "SELECT pt.created_via, parent.name
               FROM predicate_types pt
               JOIN predicate_types parent ON parent.id = pt.parent_id
              WHERE pt.name = 'orchestrates_v2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(created_via, "user_assertion");
    assert_eq!(parent_name, "relates_to");

    // Closure must include (new_id, relates_to_id).
    let closed: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM predicate_closure pc
               JOIN predicate_types child ON child.id = pc.child_id
               JOIN predicate_types ancestor ON ancestor.id = pc.ancestor_id
              WHERE child.name = 'orchestrates_v2' AND ancestor.name = 'relates_to'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(closed, 1);

    // Audit row in ontology_extension_log.
    let ext_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ontology_extension_log WHERE predicate = 'orchestrates_v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ext_count, 1);
}

#[test]
fn alias_resolution_accepts_kebab_case() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    // The kebab-case alias `has-module` is seeded by migration 0011 to
    // resolve to `has_module`. Both EXTRACTED and USER_ASSERTED must
    // accept it because the alias makes it resolvable.
    let claim_id = store
        .add_claim_from_agent(
            "parser_agent",
            AgentKind::DeterministicParser,
            "aver_core",
            "has-module",
            "extractor",
            "tree_sitter",
        )
        .expect("alias should resolve");

    let claim = store.get_claim(claim_id).unwrap();
    // ADR-0018 §"Alias-table grammar": alias is NOT rewritten on write —
    // log fidelity is preserved.
    assert_eq!(claim.predicate, "has-module");

    // The alias resolves to `has_module`, which is a child of `owns`.
    // Predicate-closure walking surfaces this row when querying `owns`.
    assert!(store.predicate_implies("has_module", "owns").unwrap());
}

#[test]
fn trigger_rejects_raw_sql_with_unknown_predicate() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).unwrap();
    drop(_store);

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let err = conn.execute(
        "INSERT INTO claims (id, subject, predicate, object, provenance, confidence, status,
                             source_refs, agent_id, agent_kind, write_ts, created_at, last_seen_at)
         VALUES (9999, 'S', 'totally_made_up', 'O', 'USER_ASSERTED', 1.0, 'ACTIVE',
                 '[\"test\"]', 'local', 'HUMAN', 1, 1, 1)",
        [],
    );

    let err = err.expect_err("trigger must reject unknown predicate");
    let msg = format!("{err}");
    assert!(
        msg.contains("claims.predicate not in ontology"),
        "expected ontology trigger message, got: {msg}"
    );
}

#[test]
fn ensure_entity_sets_requires_review_on_thing_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    assert_eq!(store.requires_review_count().unwrap(), 0);

    // `bare_subject` and `bare_object` have no `prefix:` and don't match
    // the User/Claude/Pi synonyms, so they fall back to `Thing` and must
    // be flagged for review.
    store
        .add_claim("bare_subject", "relates_to", "bare_object", "manual")
        .unwrap();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let flagged: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE requires_review = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 2);
    assert_eq!(store.requires_review_count().unwrap(), 2);

    // A typed prefix (matching a seeded entity_type) does NOT flag.
    store
        .add_claim("File:src/main.rs", "concerns", "auth", "manual")
        .unwrap();
    let typed_flagged: i64 = conn
        .query_row(
            "SELECT requires_review FROM entities WHERE name = 'File:src/main.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(typed_flagged, 0);
}
