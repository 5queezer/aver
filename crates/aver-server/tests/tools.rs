use aver_server::tools::{
    AverTools, ListCandidateClaimsParams, PromoteCandidateClaimParams, ProposeCandidateClaimParams,
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
            top_k: Some(5),
        })
        .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, remembered.id);
    assert_eq!(recalled[0].subject, "Aver");
    assert_eq!(recalled[0].agent_id, "claude");
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
