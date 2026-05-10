//! ADR-0021 — MCP-level scope on writes and scope+scope_walk on reads.
//!
//! Layer 1 surfacing: per-call `scope` is optional and defaults to `'global'`,
//! preserving today's behavior for clients that haven't been updated.

use aver_server::tools::{
    AddTripleParams, AddVectorChunkParams, AverTools, ExpandParams, ObservationRelevanceParam,
    ProposeCandidateClaimParams, RecallParams, RecordEventParams, RecordObservationParams,
    RememberClaimParams,
};

#[test]
fn remember_claim_with_scope_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let view = tools
        .remember_claim(RememberClaimParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "rusqlite".to_string(),
            source: Some("mcp-test".to_string()),
            agent_id: Some("cc".to_string()),
            agent_kind: Some("LLM".to_string()),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    assert_eq!(view.scope, "proj/aver");
}

#[test]
fn remember_claim_without_scope_defaults_to_global() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let view = tools
        .remember_claim(RememberClaimParams {
            subject: "user".to_string(),
            predicate: "prefers".to_string(),
            object: "tabs".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
            scope: None,
        })
        .unwrap();
    assert_eq!(view.scope, "global");
}

#[test]
fn add_triple_with_scope_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let view = tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "rmcp".to_string(),
            confidence: None,
            source: "mcp-test".to_string(),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    assert_eq!(view.status, "appended");
    assert!(view.triple_id > 0);
}

#[test]
fn record_event_with_scope_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let view = tools
        .record_event(RecordEventParams {
            session_id: "s1".to_string(),
            kind: "user_message".to_string(),
            payload: "hi".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    assert_eq!(view.scope, "proj/aver");
}

#[test]
fn record_observation_with_scope_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let event_view = tools
        .record_event(RecordEventParams {
            session_id: "s1".to_string(),
            kind: "user_message".to_string(),
            payload: "hi".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    let view = tools
        .record_observation(RecordObservationParams {
            session_id: "s1".to_string(),
            content: "obs body".to_string(),
            relevance: ObservationRelevanceParam::Low,
            source_event_ids: vec![event_view.id],
            derivation: "test".to_string(),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    assert_eq!(view.scope, "proj/aver");
}

#[test]
fn propose_candidate_claim_with_scope_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let event_view = tools
        .record_event(RecordEventParams {
            session_id: "s1".to_string(),
            kind: "user_message".to_string(),
            payload: "hi".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
            scope: None,
        })
        .unwrap();
    let view = tools
        .propose_candidate_claim(ProposeCandidateClaimParams {
            event_id: event_view.id,
            subject: "subj".to_string(),
            predicate: "uses".to_string(),
            object: "obj".to_string(),
            scope: Some("proj/aver/branch/feat".to_string()),
        })
        .unwrap();
    assert_eq!(view.scope, "proj/aver/branch/feat");
}

#[test]
fn recall_with_scope_filters_by_walk() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "rusqlite".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "elsewhere".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: Some("proj/other".to_string()),
        })
        .unwrap();

    let view = tools
        .recall(RecallParams {
            query: "uses".to_string(),
            alpha: None,
            hops: None,
            top_k: Some(10),
            scope: Some("proj/aver".to_string()),
            scope_walk: Some("exact".to_string()),
            agent_id: None,
            agent_kind: None,
            predicate: None,
            predicate_walk: None,
            min_confidence: None,
            status: None,
        })
        .unwrap();
    let scopes: Vec<String> = view.triples.iter().map(|c| c.scope.clone()).collect();
    assert!(
        scopes.iter().all(|s| s == "proj/aver"),
        "scopes: {scopes:?}"
    );
}

#[test]
fn expand_with_scope_filters_edges() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "in_proj".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "in_global".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: None, // → "global"
        })
        .unwrap();

    let view = tools
        .expand(ExpandParams {
            entity: "aver".to_string(),
            hops: Some(1),
            predicates: None,
            scope: Some("proj/aver".to_string()),
            scope_walk: Some("exact".to_string()),
        })
        .unwrap();
    let objects: Vec<String> = view.edges.iter().map(|c| c.object.clone()).collect();
    assert!(objects.contains(&"in_proj".to_string()));
    assert!(
        !objects.contains(&"in_global".to_string()),
        "got {objects:?}"
    );
}

#[test]
fn add_vector_chunk_does_not_take_scope_param() {
    // Vector chunks inherit scope from their claim. Adding a per-chunk scope
    // would mean a chunk could outscope its claim, which has no semantics.
    // This test pins the contract by exercising the no-scope param shape.
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let triple = tools
        .add_triple(AddTripleParams {
            subject: "aver".to_string(),
            predicate: "uses".to_string(),
            object: "x".to_string(),
            confidence: None,
            source: "t".to_string(),
            scope: Some("proj/aver".to_string()),
        })
        .unwrap();
    let _ = tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id: triple.triple_id,
            text: "chunk text".to_string(),
        })
        .unwrap();
}
