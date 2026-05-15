use aver_server::mcp::AverMcpService;
use aver_server::scopes::ALL_TOOL_NAMES;
use rmcp::ServerHandler;

#[test]
fn mcp_instructions_group_default_and_advanced_tools() {
    let dir = tempfile::tempdir().unwrap();
    let service = AverMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();
    let instructions = service.get_info().instructions.unwrap();

    assert!(instructions.contains("Aver exposes 18 MCP tools."));
    assert!(instructions.contains(
        "Start with the default memory tools: recall, expand, remember_claim, add_triple."
    ));
    assert!(instructions.contains("Use event-to-claim workflow tools only when capturing session history or reviewing staged memories: record_event, should_extract_memories, propose_candidate_claim, list_candidate_claims, promote_candidate_claim, reject_candidate_claim."));
    assert!(instructions.contains("Use observation continuity tools for source-backed session summaries and handoff state: record_observation, recall_observation, observation_coverage, assemble_compaction_summary."));
    assert!(instructions.contains("Use advanced claim-maintenance tools sparingly for contradiction handling, invalidation, consolidation, and retrieval tuning: contradict, retire_claim, consolidate, add_vector_chunk."));

    for tool_name in ALL_TOOL_NAMES {
        assert!(
            instructions.contains(tool_name),
            "missing {tool_name} from instructions"
        );
    }
}
