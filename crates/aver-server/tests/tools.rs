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
    assert_eq!(recalled.confidence_floor, 0.0);

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
