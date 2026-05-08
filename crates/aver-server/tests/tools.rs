use aver_server::tools::{
    AddTripleParams, AverTools, ConsolidateParams, ContradictParams, ExpandParams,
    ListCandidateClaimsParams, PromoteCandidateClaimParams, ProposeCandidateClaimParams,
    RecallParams, RecordEventParams, RejectCandidateClaimParams, RememberClaimParams,
    ShouldExtractMemoriesParams,
};

#[test]
fn remember_claim_tool_writes_claim_and_recall_returns_it() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let remembered = tools
        .remember_claim(RememberClaimParams {
            subject: "Aver".to_string(),
            predicate: "exposes".to_string(),
            object: "MCP_tools".to_string(),
            source: Some("mcp-test".to_string()),
            agent_id: Some("claude".to_string()),
            agent_kind: Some("LLM".to_string()),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "MCP_tools".to_string(),
            alpha: None,
            hops: None,
            top_k: Some(5),
        })
        .unwrap();

    assert_eq!(recalled.triples.len(), 1);
    assert_eq!(recalled.triples[0].id, remembered.id);
    assert_eq!(recalled.triples[0].subject, "Aver");
    assert_eq!(recalled.triples[0].agent_id, "claude");
}

#[test]
fn adr0008_five_tool_surface_covers_claim_graph_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let triple = tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: None,
            source: "agent-test".to_string(),
        })
        .unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "StripeSDK".to_string(),
            predicate: "owned_by".to_string(),
            object: "BillingTeam".to_string(),
            confidence: None,
            source: "agent-test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "Stripe".to_string(),
            alpha: None,
            hops: Some(2),
            top_k: Some(5),
        })
        .unwrap();
    assert!(
        recalled
            .triples
            .iter()
            .any(|claim| claim.id == triple.triple_id)
    );
    assert_eq!(recalled.confidence_floor, 0.95);

    let expanded = tools
        .expand(ExpandParams {
            entity: "PaymentGateway".to_string(),
            hops: Some(2),
            predicates: None,
        })
        .unwrap();
    assert!(expanded.nodes.contains(&"BillingTeam".to_string()));

    let contradiction = tools
        .contradict(ContradictParams {
            triple_id: triple.triple_id,
            reason: "vendor migration completed".to_string(),
            new_claim: None,
        })
        .unwrap();
    assert_eq!(contradiction.status, "recorded");

    let consolidated = tools
        .consolidate(ConsolidateParams { scope: None })
        .unwrap();
    assert_eq!(consolidated.decayed, 1);
}

#[test]
fn episodic_candidate_tools_cover_memory_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let event = tools
        .record_event(RecordEventParams {
            session_id: "session-1".to_string(),
            kind: "explicit_remember".to_string(),
            payload: "Aver prefers triggered memory extraction".to_string(),
            source: Some("mcp-test".to_string()),
            agent_id: Some("claude".to_string()),
            agent_kind: Some("LLM".to_string()),
        })
        .unwrap();

    let should_extract = tools
        .should_extract_memories(ShouldExtractMemoriesParams {
            session_id: "session-1".to_string(),
            event_threshold: 10,
        })
        .unwrap();
    assert!(should_extract.should_extract);

    let candidate = tools
        .propose_candidate_claim(ProposeCandidateClaimParams {
            event_id: event.id,
            subject: "Aver".to_string(),
            predicate: "prefers".to_string(),
            object: "triggered_memory_extraction".to_string(),
        })
        .unwrap();

    let candidates = tools
        .list_candidate_claims(ListCandidateClaimsParams {
            session_id: Some("session-1".to_string()),
            status: Some("PENDING".to_string()),
        })
        .unwrap();
    assert_eq!(candidates, vec![candidate.clone()]);

    tools
        .reject_candidate_claim(RejectCandidateClaimParams {
            candidate_id: candidate.id,
            reason: "duplicate".to_string(),
        })
        .unwrap();
    let rejected = tools
        .list_candidate_claims(ListCandidateClaimsParams {
            session_id: Some("session-1".to_string()),
            status: Some("REJECTED".to_string()),
        })
        .unwrap();
    assert_eq!(rejected[0].rejection_reason.as_deref(), Some("duplicate"));

    let second = tools
        .propose_candidate_claim(ProposeCandidateClaimParams {
            event_id: event.id,
            subject: "Aver".to_string(),
            predicate: "stores".to_string(),
            object: "candidate_claims".to_string(),
        })
        .unwrap();
    let promoted = tools
        .promote_candidate_claim(PromoteCandidateClaimParams {
            candidate_id: second.id,
        })
        .unwrap();
    assert_eq!(promoted.subject, "Aver");
    assert_eq!(promoted.source_refs, [format!("event:{}", event.id)]);
}

#[test]
fn recall_tool_rejects_alpha_outside_unit_interval() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .recall(RecallParams {
            query: "Stripe".to_string(),
            alpha: Some(1.5),
            hops: None,
            top_k: Some(5),
        })
        .expect_err("invalid alpha should be rejected");

    assert!(err.to_string().contains("alpha"));
}

#[test]
fn add_triple_rejects_confidence_outside_unit_interval() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: Some(-0.1),
            source: "agent-test".to_string(),
        })
        .expect_err("invalid confidence should be rejected");

    assert!(err.to_string().contains("confidence"));
}

#[test]
fn add_triple_persists_valid_confidence_override() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let written = tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: Some(0.4),
            source: "agent-test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "PaymentGateway".to_string(),
            alpha: None,
            hops: None,
            top_k: Some(5),
        })
        .unwrap();

    let claim = recalled
        .triples
        .iter()
        .find(|claim| claim.id == written.triple_id)
        .expect("written claim should be recalled");
    assert_eq!(claim.confidence, 0.4);
}

#[test]
fn expand_tool_rejects_zero_hops() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .expand(ExpandParams {
            entity: "PaymentGateway".to_string(),
            hops: Some(0),
            predicates: None,
        })
        .expect_err("zero hops should be rejected");

    assert!(err.to_string().contains("hops"));
}

#[test]
fn recall_tool_rejects_zero_hops() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .recall(RecallParams {
            query: "PaymentGateway".to_string(),
            alpha: None,
            hops: Some(0),
            top_k: Some(5),
        })
        .expect_err("zero hops should be rejected");

    assert!(err.to_string().contains("hops"));
}

#[test]
fn recall_tool_rejects_zero_top_k() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .recall(RecallParams {
            query: "PaymentGateway".to_string(),
            alpha: None,
            hops: None,
            top_k: Some(0),
        })
        .expect_err("zero top_k should be rejected");

    assert!(err.to_string().contains("top_k"));
}

#[test]
fn recall_tool_returns_graph_context_for_entity_query() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: None,
            source: "test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "PaymentGateway".to_string(),
            alpha: None,
            hops: Some(1),
            top_k: Some(5),
        })
        .unwrap();

    assert!(
        recalled
            .subgraph
            .nodes
            .contains(&"PaymentGateway".to_string())
    );
    assert!(recalled.subgraph.nodes.contains(&"StripeSDK".to_string()));
    assert_eq!(recalled.subgraph.edges.len(), 1);
    assert_eq!(recalled.subgraph.edges[0].predicate, "depends_on");
}

#[test]
fn recall_tool_expands_graph_from_recalled_claim_subject_when_query_is_phrase() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: None,
            source: "test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "what depends on PaymentGateway".to_string(),
            alpha: None,
            hops: Some(1),
            top_k: Some(5),
        })
        .unwrap();

    assert_eq!(recalled.triples.len(), 1);
    assert!(
        recalled
            .subgraph
            .nodes
            .contains(&"PaymentGateway".to_string())
    );
    assert!(recalled.subgraph.nodes.contains(&"StripeSDK".to_string()));
    assert_eq!(recalled.subgraph.edges.len(), 1);
}

#[test]
fn recall_tool_reports_confidence_floor_for_returned_triples() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "status".to_string(),
            object: "current".to_string(),
            confidence: Some(0.4),
            source: "test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "PaymentGateway".to_string(),
            alpha: None,
            hops: Some(1),
            top_k: Some(5),
        })
        .unwrap();

    assert_eq!(recalled.triples.len(), 1);
    assert_eq!(recalled.confidence_floor, 0.4);
}

#[test]
fn recall_tool_confidence_floor_includes_subgraph_edges() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "status".to_string(),
            object: "current".to_string(),
            confidence: Some(0.95),
            source: "test".to_string(),
        })
        .unwrap();
    tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: Some(0.4),
            source: "test".to_string(),
        })
        .unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "status current".to_string(),
            alpha: None,
            hops: Some(1),
            top_k: Some(1),
        })
        .unwrap();

    assert_eq!(recalled.triples.len(), 1);
    assert_eq!(recalled.subgraph.edges.len(), 2);
    assert_eq!(recalled.confidence_floor, 0.4);
}

#[test]
fn consolidate_tool_rejects_unsupported_scope() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .consolidate(ConsolidateParams {
            scope: Some("session:abc".to_string()),
        })
        .expect_err("unsupported scoped consolidation should be explicit");

    assert!(err.to_string().contains("scope"));
}

#[test]
fn should_extract_memories_rejects_zero_event_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .should_extract_memories(ShouldExtractMemoriesParams {
            session_id: "s1".to_string(),
            event_threshold: 0,
        })
        .expect_err("zero threshold should be rejected");

    assert!(err.to_string().contains("event_threshold"));
}

#[test]
fn record_event_tool_rejects_empty_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .record_event(RecordEventParams {
            session_id: "".to_string(),
            kind: "message".to_string(),
            payload: "hello".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
        })
        .expect_err("empty session ids should be rejected");

    assert!(err.to_string().contains("session_id"));
}

#[test]
fn record_event_tool_rejects_empty_kind() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .record_event(RecordEventParams {
            session_id: "s1".to_string(),
            kind: " ".to_string(),
            payload: "hello".to_string(),
            source: None,
            agent_id: None,
            agent_kind: None,
        })
        .expect_err("empty event kinds should be rejected");

    assert!(err.to_string().contains("kind"));
}

#[test]
fn add_triple_rejects_empty_subject() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .add_triple(AddTripleParams {
            subject: " ".to_string(),
            predicate: "depends_on".to_string(),
            object: "StripeSDK".to_string(),
            confidence: None,
            source: "test".to_string(),
        })
        .expect_err("empty subjects should be rejected");

    assert!(err.to_string().contains("subject"));
}

#[test]
fn add_triple_rejects_empty_predicate() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: " ".to_string(),
            object: "StripeSDK".to_string(),
            confidence: None,
            source: "test".to_string(),
        })
        .expect_err("empty predicates should be rejected");

    assert!(err.to_string().contains("predicate"));
}

#[test]
fn add_triple_rejects_empty_object() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .add_triple(AddTripleParams {
            subject: "PaymentGateway".to_string(),
            predicate: "depends_on".to_string(),
            object: " ".to_string(),
            confidence: None,
            source: "test".to_string(),
        })
        .expect_err("empty objects should be rejected");

    assert!(err.to_string().contains("object"));
}
