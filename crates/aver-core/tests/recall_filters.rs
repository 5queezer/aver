//! ADR-0023 — typed recall/expand filters: agent_id, agent_kind,
//! predicate + predicate_walk, min_confidence, status.

use aver_core::{AgentKind, ClaimStatus, PredicateWalk, RecallFilters, ScopeWalk, Store};

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

#[test]
fn recall_with_agent_id_filter_excludes_others() {
    let (_dir, store) = open_store();
    store
        .add_claim_from_agent("alice", AgentKind::Human, "x", "uses", "y", "test")
        .unwrap();
    store
        .add_claim_from_agent("bob", AgentKind::Human, "x", "uses", "z", "test")
        .unwrap();
    let claims = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                agent_id: Some("alice".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(claims.iter().all(|c| c.agent_id == "alice"));
    assert!(claims.iter().any(|c| c.object == "y"));
    assert!(!claims.iter().any(|c| c.object == "z"));
}

#[test]
fn recall_with_agent_kind_filter_excludes_others() {
    let (_dir, store) = open_store();
    store
        .add_claim_from_agent("a", AgentKind::Human, "x", "uses", "y", "t")
        .unwrap();
    store
        .add_claim_from_agent("b", AgentKind::Llm, "x", "uses", "z", "t")
        .unwrap();
    let claims = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                agent_kind: Some(AgentKind::Human),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(claims.iter().all(|c| c.agent_kind == AgentKind::Human));
}

#[test]
fn recall_with_min_confidence_filter() {
    let (_dir, store) = open_store();
    store
        .add_claim_with_confidence("x", "uses", "high", "t", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("x", "uses", "low", "t", 0.40)
        .unwrap();
    let claims = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                min_confidence: Some(0.8),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(claims.iter().all(|c| c.confidence >= 0.8));
    assert!(!claims.iter().any(|c| c.object == "low"));
}

#[test]
fn recall_with_status_any_includes_invalidated() {
    let (_dir, store) = open_store();
    let id = store.add_claim("x", "uses", "y", "t").unwrap();
    store.retire_claim(id, "test retirement").unwrap();
    // Default status filter is ACTIVE → should NOT see this row.
    let active = store
        .recall_text_with_filters("uses", RecallFilters::default())
        .unwrap();
    assert!(!active.iter().any(|c| c.id == id));
    // status: None means "any" → should see it.
    let any = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                status: None,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(any.iter().any(|c| c.id == id));
}

#[test]
fn recall_with_predicate_filter_exact() {
    let (_dir, store) = open_store();
    store.add_claim("aver", "uses", "rusqlite", "t").unwrap();
    store.add_claim("aver", "imports", "x", "t").unwrap();
    let claims = store
        .recall_text_with_filters(
            "aver",
            RecallFilters {
                predicate: Some("uses".to_string()),
                predicate_walk: PredicateWalk::Exact,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(claims.iter().all(|c| c.predicate == "uses"));
}

#[test]
fn recall_with_predicate_filter_descendants_uses_closure() {
    // ADR-0010 closure: `imports` and `calls` are children of `depends_on`.
    let (_dir, store) = open_store();
    store.add_claim("aver", "imports", "x", "t").unwrap();
    store.add_claim("aver", "calls", "y", "t").unwrap();
    store.add_claim("aver", "owns", "z", "t").unwrap();
    let claims = store
        .recall_text_with_filters(
            "aver",
            RecallFilters {
                predicate: Some("depends_on".to_string()),
                predicate_walk: PredicateWalk::Descendants,
                ..Default::default()
            },
        )
        .unwrap();
    let preds: Vec<String> = claims.iter().map(|c| c.predicate.clone()).collect();
    assert!(preds.iter().any(|p| p == "imports"), "{preds:?}");
    assert!(preds.iter().any(|p| p == "calls"), "{preds:?}");
    assert!(!preds.iter().any(|p| p == "owns"), "{preds:?}");
}

#[test]
fn recall_filters_compose() {
    let (_dir, store) = open_store();
    store
        .add_claim_from_agent("alice", AgentKind::Llm, "aver", "uses", "rusqlite", "t")
        .unwrap();
    store
        .add_claim_from_agent("bob", AgentKind::Human, "aver", "uses", "rmcp", "t")
        .unwrap();
    let claims = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                agent_id: Some("alice".to_string()),
                agent_kind: Some(AgentKind::Llm),
                predicate: Some("uses".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].object, "rusqlite");
}

#[test]
fn recall_filters_default_preserves_today_behavior() {
    // RecallFilters::default() reproduces the pre-Layer-3 behavior of
    // `recall_text` exactly: scope="global", walk=any, ACTIVE only.
    let (_dir, store) = open_store();
    store.add_claim("x", "uses", "y", "t").unwrap();
    let claims = store
        .recall_text_with_filters("uses", RecallFilters::default())
        .unwrap();
    assert!(!claims.is_empty());
    assert!(claims.iter().all(|c| c.status == ClaimStatus::Active));
}

#[test]
fn recall_filters_min_confidence_validates_range() {
    let (_dir, store) = open_store();
    let err = store
        .recall_text_with_filters(
            "x",
            RecallFilters {
                min_confidence: Some(1.5),
                ..Default::default()
            },
        )
        .expect_err("min_confidence > 1 must reject");
    let _ = err;
}

#[test]
fn scope_filter_composes_with_typed_filters() {
    let (_dir, store) = open_store();
    store
        .add_claim_with_scope("x", "uses", "in_proj", "t", "proj/aver")
        .unwrap();
    store.add_claim("x", "uses", "in_global", "t").unwrap();
    let claims = store
        .recall_text_with_filters(
            "uses",
            RecallFilters {
                scope: "proj/aver".to_string(),
                scope_walk: ScopeWalk::Exact,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(claims.iter().all(|c| c.scope == "proj/aver"));
    assert!(claims.iter().any(|c| c.object == "in_proj"));
    assert!(!claims.iter().any(|c| c.object == "in_global"));
}
