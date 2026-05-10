//! ADR-0021 Layer 1 — scope-aware reads on `recall_text` and `expand`.
//!
//! Walk modes per ADR-0021 §"Read-path semantics":
//! - exact:       only rows whose scope equals input
//! - ancestors:   input scope + every path prefix up to "global"
//! - descendants: input scope + every path that starts with `input/`
//! - any:         no filter
//!
//! `recall_text` and `expand` keep their default (today's) behavior when
//! callers don't pass a scope. New `_with_scope` variants accept a scope
//! and walk mode.

use aver_core::{ScopeWalk, Store};

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn seed_corpus(store: &Store) {
    store
        .add_claim_with_scope("user", "prefers", "tabs", "test", "global")
        .unwrap();
    store
        .add_claim_with_scope("aver", "uses", "rusqlite", "test", "proj/aver")
        .unwrap();
    store
        .add_claim_with_scope("aver", "uses", "rmcp", "test", "proj/aver/branch/feat_x")
        .unwrap();
    store
        .add_claim_with_scope("vasudev", "uses", "torch", "test", "proj/vasudev")
        .unwrap();
}

#[test]
fn recall_text_default_scope_is_any_walk() {
    let (_dir, store) = open_store();
    seed_corpus(&store);
    // Default `recall_text` preserves pre-scope behavior (Layer 1): returns
    // every active claim regardless of scope.
    let claims = store.recall_text("uses").unwrap();
    let scopes: Vec<String> = claims.iter().map(|c| c.scope.clone()).collect();
    assert!(!scopes.contains(&"global".to_string())); // "user prefers tabs" doesn't match "uses"
    assert!(scopes.contains(&"proj/aver".to_string()));
    assert!(scopes.contains(&"proj/aver/branch/feat_x".to_string()));
    assert!(scopes.contains(&"proj/vasudev".to_string()));
}

#[test]
fn recall_text_with_scope_exact_returns_only_that_scope() {
    let (_dir, store) = open_store();
    seed_corpus(&store);
    let claims = store
        .recall_text_with_scope("uses", "proj/aver", ScopeWalk::Exact)
        .unwrap();
    let subjects: Vec<String> = claims.iter().map(|c| c.subject.clone()).collect();
    let scopes: Vec<String> = claims.iter().map(|c| c.scope.clone()).collect();
    assert!(
        scopes.iter().all(|s| s == "proj/aver"),
        "scopes: {scopes:?}"
    );
    assert!(subjects.contains(&"aver".to_string()));
    assert!(!subjects.contains(&"vasudev".to_string()));
}

#[test]
fn recall_text_with_scope_ancestors_includes_global_and_path_prefixes() {
    let (_dir, store) = open_store();
    seed_corpus(&store);
    // Add a "global" claim that the query can match.
    store
        .add_claim_with_scope("aver", "is", "tool", "test", "global")
        .unwrap();
    let claims = store
        .recall_text_with_scope("aver", "proj/aver", ScopeWalk::Ancestors)
        .unwrap();
    let scopes: Vec<String> = claims.iter().map(|c| c.scope.clone()).collect();
    assert!(scopes.contains(&"proj/aver".to_string()));
    assert!(scopes.contains(&"global".to_string()));
    // proj/aver/branch/feat_x is a descendant, NOT an ancestor → excluded.
    assert!(
        !scopes.contains(&"proj/aver/branch/feat_x".to_string()),
        "ancestors walk must not include descendants; got {scopes:?}"
    );
    // proj/vasudev is a sibling, NOT an ancestor → excluded.
    assert!(
        !scopes.contains(&"proj/vasudev".to_string()),
        "got {scopes:?}"
    );
}

#[test]
fn recall_text_with_scope_descendants_includes_subpaths() {
    let (_dir, store) = open_store();
    seed_corpus(&store);
    let claims = store
        .recall_text_with_scope("uses", "proj/aver", ScopeWalk::Descendants)
        .unwrap();
    let scopes: Vec<String> = claims.iter().map(|c| c.scope.clone()).collect();
    assert!(scopes.contains(&"proj/aver".to_string()));
    assert!(scopes.contains(&"proj/aver/branch/feat_x".to_string()));
    // global and proj/vasudev are not under proj/aver → excluded.
    assert!(
        !scopes.contains(&"proj/vasudev".to_string()),
        "got {scopes:?}"
    );
}

#[test]
fn recall_text_with_scope_any_returns_everything() {
    let (_dir, store) = open_store();
    seed_corpus(&store);
    let claims = store
        .recall_text_with_scope("uses", "proj/aver", ScopeWalk::Any)
        .unwrap();
    assert!(
        claims.len() >= 3,
        "any walk must return all active claims for query"
    );
}

#[test]
fn recall_text_with_scope_global_ancestors_is_just_global() {
    let (_dir, store) = open_store();
    store
        .add_claim_with_scope("aver", "is", "tool", "test", "global")
        .unwrap();
    store
        .add_claim_with_scope("aver", "uses", "x", "test", "proj/aver")
        .unwrap();
    let claims = store
        .recall_text_with_scope("aver", "global", ScopeWalk::Ancestors)
        .unwrap();
    let scopes: Vec<String> = claims.iter().map(|c| c.scope.clone()).collect();
    assert!(scopes.iter().all(|s| s == "global"), "scopes: {scopes:?}");
}

#[test]
fn expand_with_scope_filters_edges_by_scope() {
    let (_dir, store) = open_store();
    // Two graphs; same entity name but different scopes.
    store
        .add_claim_with_scope("X", "uses", "Y_global", "test", "global")
        .unwrap();
    store
        .add_claim_with_scope("X", "uses", "Y_proj", "test", "proj/aver")
        .unwrap();
    let graph = store
        .expand_with_scope("X", 1, None, "global", ScopeWalk::Exact)
        .unwrap();
    let edge_objects: Vec<String> = graph.edges.iter().map(|e| e.object.clone()).collect();
    assert!(edge_objects.contains(&"Y_global".to_string()));
    assert!(
        !edge_objects.contains(&"Y_proj".to_string()),
        "got {edge_objects:?}"
    );
}

#[test]
fn invalid_scope_walk_input_validates_via_scope_charset() {
    let (_dir, store) = open_store();
    let err = store
        .recall_text_with_scope("foo", "bad@char", ScopeWalk::Exact)
        .expect_err("invalid scope must reject");
    assert!(format!("{err:?}").to_lowercase().contains("scope"));
}
