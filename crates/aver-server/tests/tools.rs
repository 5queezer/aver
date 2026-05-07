use aver_server::tools::{AverTools, RecallParams, RememberClaimParams};

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
