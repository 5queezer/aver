use aver_server::mcp::AverMcpService;
use aver_server::scopes::ALL_TOOL_NAMES;
use rmcp::ServerHandler;

#[test]
fn mcp_instructions_group_primary_and_advanced_tools() {
    let dir = tempfile::tempdir().unwrap();
    let service = AverMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();
    let instructions = service.get_info().instructions.unwrap();

    assert!(instructions.contains("Aver exposes 18 MCP tools."));
    assert!(instructions.contains(
        "Primary tools: recall, remember_claim, record_event, record_observation, assemble_compaction_summary."
    ));
    assert!(instructions.contains(
        "Decision policy: start with recall when you need existing durable memory; use remember_claim only for explicit long-term facts; use record_event for raw session history before extraction; use record_observation and assemble_compaction_summary for source-backed handoff state."
    ));
    assert!(instructions.contains(
        "Default workflows: recall existing memory before answering or updating; record_event -> should_extract_memories -> propose_candidate_claim/list_candidate_claims -> promote_candidate_claim or reject_candidate_claim for event-to-claim promotion; record_observation -> recall_observation or observation_coverage -> assemble_compaction_summary for continuity and compaction."
    ));
    assert!(instructions.contains(
        "Use event-to-claim workflow tools only when capturing session history or reviewing staged memories: record_event, should_extract_memories, propose_candidate_claim, list_candidate_claims, promote_candidate_claim, reject_candidate_claim."
    ));
    assert!(instructions.contains(
        "Use observation continuity tools for source-backed session summaries and handoff state: record_observation, recall_observation, observation_coverage, assemble_compaction_summary."
    ));
    assert!(instructions.contains(
        "Use advanced claim-maintenance tools sparingly for contradiction handling, invalidation, consolidation, and retrieval tuning: contradict, retire_claim, consolidate, add_vector_chunk."
    ));

    for tool_name in ALL_TOOL_NAMES {
        assert!(
            instructions.contains(tool_name),
            "missing {tool_name} from instructions"
        );
    }
}
