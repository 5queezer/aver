//! T2 — adding a claim writes to the JSONL log first (source of truth,
//! ADR-0005) and mirrors into SQLite. Both views are queryable.
//! T3 — get_claim returns full ADR-0003 metadata: provenance,
//! confidence, status, and source_refs.

use aver_core::{ClaimStatus, Provenance, Store};

#[test]
fn add_claim_appends_to_episodic_log_and_mirrors_to_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .expect("add_claim should succeed");

    // SQLite mirror: claim is queryable by id, with the right s/p/o.
    let row = store
        .get_claim(claim_id)
        .expect("get_claim should return the row");
    assert_eq!(row.subject, "auth_service");
    assert_eq!(row.predicate, "depends_on");
    assert_eq!(row.object, "stripe_sdk");

    // JSONL log: the write was appended.
    let log_path = dir.path().join("log.jsonl");
    let log = std::fs::read_to_string(&log_path).expect("log.jsonl should exist");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one log line for one claim");

    let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["kind"], "add_claim");
    assert_eq!(entry["subject"], "auth_service");
    assert_eq!(entry["predicate"], "depends_on");
    assert_eq!(entry["object"], "stripe_sdk");
    assert_eq!(entry["source"], "test_session");
}

#[test]
fn get_claim_returns_provenance_confidence_status_and_source_refs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let id = store
        .add_claim("a", "rel", "b", "test_session")
        .expect("add_claim should succeed");
    let c = store
        .get_claim(id)
        .expect("get_claim should return the row");

    assert_eq!(c.provenance, Provenance::UserAsserted);
    assert!(
        (c.confidence - 0.95).abs() < f64::EPSILON,
        "default confidence for USER_ASSERTED should be 0.95 (ADR-0003)"
    );
    assert_eq!(c.status, ClaimStatus::Active);
    assert_eq!(c.source_refs, vec!["test_session".to_string()]);
}

/// T4 — log-first invariant is *replay-safe*: the JSONL entry carries the
/// same claim_id as the SQLite row, so a fresh DB rebuilt from the log
/// would assign matching ids. Closes the audit-replay gap pi flagged in
/// the T2 review.
#[test]
fn add_claim_log_entry_claim_id_matches_sqlite_row_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("a", "rel", "b", "test_session")
        .expect("add_claim should succeed");

    // SQLite row's id matches the value returned by add_claim.
    let row = store.get_claim(claim_id).unwrap();
    assert_eq!(row.id, claim_id);

    // The JSONL log entry carries the same claim_id.
    let log_path = dir.path().join("log.jsonl");
    let log = std::fs::read_to_string(&log_path).unwrap();
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(entry["claim_id"], serde_json::json!(claim_id));
}

/// T5 — minimal recall: keyword search over claim text returns matching
/// active claims. This is the v0.1 text-only precursor to HybridRAG recall
/// in ADR-0004 / ADR-0008.
#[test]
fn recall_text_returns_claim_by_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let auth_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .expect("add auth claim");
    store
        .add_claim("billing_worker", "emits", "invoice_event", "test_session")
        .expect("add billing claim");

    let matches = store
        .recall_text("stripe")
        .expect("recall_text should search stored claims");

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, auth_id);
    assert_eq!(matches[0].subject, "auth_service");
    assert_eq!(matches[0].object, "stripe_sdk");
}

#[test]
fn recall_text_ranks_claims_by_multi_token_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let language_id = store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    let name_id = store
        .add_claim("project", "name", "Aver", "test_session")
        .unwrap();
    store
        .add_claim("meeting", "attendee", "Bob", "test_session")
        .unwrap();

    let matches = store.recall_text("Aver project").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![name_id, language_id]
    );
}

#[test]
fn recall_text_filters_weak_compound_token_distractors() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let language_id = store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    let name_id = store
        .add_claim("project", "name", "Aver", "test_session")
        .unwrap();
    store
        .add_claim("project", "old_language", "Python", "test_session")
        .unwrap();

    let matches = store.recall_text("Aver language").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![language_id, name_id]
    );
}

#[test]
fn recall_text_prefers_claims_covering_multiple_query_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    store
        .add_claim("user", "name", "Alice", "test_session")
        .unwrap();
    let user_rust_id = store
        .add_claim("user", "likes", "Rust", "test_session")
        .unwrap();

    let matches = store.recall_text("Rust user").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![user_rust_id]
    );
}

#[test]
fn recall_text_ranks_ambiguous_single_token_queries_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let project_rust_id = store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    let user_rust_id = store
        .add_claim("user", "likes", "Rust", "test_session")
        .unwrap();
    let benchmark_rust_id = store
        .add_claim("benchmark", "language", "Rust", "test_session")
        .unwrap();

    let matches = store.recall_text("Rust").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![project_rust_id, user_rust_id, benchmark_rust_id]
    );
}

#[test]
fn recall_text_preserves_multi_answer_single_token_matches() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let first = store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    let second = store
        .add_claim("user", "likes", "Rust", "test_session")
        .unwrap();
    let third = store
        .add_claim("benchmark", "language", "Rust", "test_session")
        .unwrap();

    let matches = store.recall_text("Rust").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![first, second, third]
    );
}

#[test]
fn recall_text_singularizes_plural_query_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let project_rust_id = store
        .add_claim("project", "language", "Rust", "test_session")
        .unwrap();
    store
        .add_claim("user", "likes", "Rust", "test_session")
        .unwrap();

    let matches = store.recall_text("projects using Rust").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![project_rust_id]
    );
}

#[test]
fn recall_text_splits_camel_case_claim_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let bench_name_id = store
        .add_claim("benchmark", "name", "MemoryAgentBench", "test_session")
        .unwrap();
    store
        .add_claim("benchmark", "topic", "memory_recall", "test_session")
        .unwrap();

    let matches = store.recall_text("memory agent bench benchmark").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![bench_name_id]
    );
}

#[test]
fn recall_text_matches_attendee_role_from_attends_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let attendee_id = store
        .add_claim("meeting", "attendee", "Bob", "test_session")
        .unwrap();
    store
        .add_claim("meeting", "topic", "project_planning", "test_session")
        .unwrap();

    let matches = store.recall_text("who attends meetings").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![attendee_id]
    );
}

#[test]
fn recall_text_matches_acronym_claim_from_expanded_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let mcp_id = store
        .add_claim("protocol", "name", "MCP", "test_session")
        .unwrap();
    store
        .add_claim("protocol", "transport", "HTTP", "test_session")
        .unwrap();

    let matches = store.recall_text("model context protocol").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![mcp_id]
    );
}

#[test]
fn recall_text_matches_camel_case_identifier_from_acronym_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let bench_name_id = store
        .add_claim("benchmark", "name", "MemoryAgentBench", "test_session")
        .unwrap();
    store
        .add_claim("benchmark", "topic", "memory_recall", "test_session")
        .unwrap();

    let matches = store.recall_text("MAB benchmark").unwrap();

    assert_eq!(
        matches.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![bench_name_id]
    );
}

#[test]
fn claim_text_renders_subject_predicate_object_for_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();

    let claim = store.get_claim(claim_id).unwrap();

    assert_eq!(claim.text(), "auth_service depends_on stripe_sdk");
}

#[test]
fn add_claim_records_default_agent_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("shared_mode", "tracks", "agent_provenance", "test_session")
        .unwrap();

    let claim = store.get_claim(claim_id).unwrap();

    assert_eq!(claim.agent_id, "local");
    assert_eq!(claim.agent_kind.as_str(), "HUMAN");
    assert!(claim.write_ts > 0);
}

#[test]
fn add_claim_from_agent_records_explicit_agent_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim_from_agent(
            "parser_agent",
            aver_core::AgentKind::DeterministicParser,
            "module",
            "defines",
            "SharedStore",
            "tree_sitter",
        )
        .unwrap();

    let claim = store.get_claim(claim_id).unwrap();

    assert_eq!(claim.agent_id, "parser_agent");
    assert_eq!(claim.agent_kind, aver_core::AgentKind::DeterministicParser);
    assert!(claim.write_ts > 0);
}

#[test]
fn add_claim_from_agent_appends_to_agent_partition_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim_from_agent(
            "parser_agent",
            aver_core::AgentKind::DeterministicParser,
            "module",
            "defines",
            "SharedStore",
            "tree_sitter",
        )
        .unwrap();

    let log_path = dir.path().join("agents/parser_agent/log.jsonl");
    let log = std::fs::read_to_string(log_path).expect("agent partition log should exist");
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();

    assert_eq!(entry["claim_id"], serde_json::json!(claim_id));
    assert_eq!(entry["agent_id"], "parser_agent");
    assert_eq!(entry["agent_kind"], "DETERMINISTIC_PARSER");
}

#[test]
fn add_claim_from_agent_rejects_path_like_agent_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .add_claim_from_agent(
            "../escape",
            aver_core::AgentKind::ExternalTool,
            "module",
            "defines",
            "UnsafeAgent",
            "test_session",
        )
        .unwrap_err();

    assert!(matches!(err, aver_core::Error::InvalidAgentId { .. }));
    assert!(!dir.path().join("escape/log.jsonl").exists());
}

#[test]
fn consolidate_supersedes_duplicate_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store.add_claim("a", "rel", "b", "s1").unwrap();
    let duplicate = store.add_claim("a", "rel", "b", "s2").unwrap();

    let superseded = store.consolidate().unwrap();

    assert_eq!(superseded, 1);
    assert_eq!(store.get_claim(first).unwrap().status, ClaimStatus::Active);
    assert_eq!(
        store.get_claim(duplicate).unwrap().status,
        ClaimStatus::Superseded
    );
}

#[test]
fn consolidate_merges_duplicate_source_refs_into_survivor() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store.add_claim("a", "rel", "b", "s1").unwrap();
    store.add_claim("a", "rel", "b", "s2").unwrap();

    store.consolidate().unwrap();

    assert_eq!(
        store.get_claim(first).unwrap().source_refs,
        vec!["s1".to_string(), "s2".to_string()]
    );
}

#[test]
fn consolidate_supersedes_older_conflicting_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let older = store
        .add_claim("feature", "status", "planned", "s1")
        .unwrap();
    let newer = store
        .add_claim("feature", "status", "shipped", "s2")
        .unwrap();

    let superseded = store.consolidate().unwrap();

    assert_eq!(superseded, 1);
    assert_eq!(
        store.get_claim(older).unwrap().status,
        ClaimStatus::Superseded
    );
    assert_eq!(store.get_claim(newer).unwrap().status, ClaimStatus::Active);
}
