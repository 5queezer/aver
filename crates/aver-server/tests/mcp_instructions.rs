use aver_server::mcp::AverMcpService;
use aver_server::scopes::ALL_TOOL_NAMES;
use aver_server::tools::{RecallParams, RecordEventParams, RememberClaimParams};
use rmcp::ServerHandler;
use schemars::schema_for;
use serde_json::Value;

#[test]
fn mcp_instructions_are_a_progressive_discovery_card() {
    let dir = tempfile::tempdir().unwrap();
    let service = AverMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();
    let instructions = service.get_info().instructions.unwrap();

    assert!(instructions.contains(
        "Aver exposes 18 MCP tools, but most sessions should start with the default active set."
    ));
    assert!(instructions.contains(
        "Default active set: recall, remember_claim, record_event, record_observation, assemble_compaction_summary."
    ));
    assert!(instructions.contains(
        "Progressively load event-to-claim tools only when converting raw events into reviewed durable claims: should_extract_memories, propose_candidate_claim, list_candidate_claims, promote_candidate_claim, reject_candidate_claim."
    ));
    assert!(instructions.contains(
        "unknown-predicate errors include fuzzy/semantic suggestions, accepted aliases, and a retry hint"
    ));
    assert!(instructions.contains(
        "Progressively load graph navigation tools only after recall returns an entity or you already know an anchor: expand; use add_triple instead of remember_claim only when confidence/source control is required."
    ));
    assert!(instructions.contains(
        "Progressively load observation audit tools only for handoff, compaction, or provenance checks: recall_observation, observation_coverage."
    ));
    assert!(instructions.contains(
        "Keep maintenance tools hidden until there is an explicit repair need: contradict, retire_claim, consolidate, add_vector_chunk."
    ));
    assert!(instructions.to_ascii_lowercase().contains(
        "only record identity details when they are necessary, user-shared, and not sensitive personal data"
    ));
    assert!(instructions.to_ascii_lowercase().contains(
        "avoid storing secrets, credentials, sensitive personal data, transient chat, or facts you cannot explain with provenance"
    ));

    for tool_name in ALL_TOOL_NAMES {
        assert!(
            instructions.contains(tool_name),
            "missing {tool_name} from instructions"
        );
    }
}

#[test]
fn primary_tool_schema_docs_explain_selection_and_safe_defaults() {
    let remember = schema_value::<RememberClaimParams>();
    assert_property_description_contains(
        &remember,
        "scope",
        "Defaults to the caller's resolved scope",
    );
    assert_property_description_contains(&remember, "source", "provenance");

    let recall = schema_value::<RecallParams>();
    assert_property_description_contains(&recall, "query", "Start broad enough");
    assert_property_description_contains(&recall, "top_k", "Defaults to 5");
    assert_property_description_contains(&recall, "scope_walk", "Use exact");

    let record_event = schema_value::<RecordEventParams>();
    assert_property_description_contains(&record_event, "payload", "raw session content");
    assert_property_description_contains(&record_event, "kind", "stable event label");
}

fn schema_value<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("schema should serialize")
}

fn assert_property_description_contains(schema: &Value, property: &str, expected: &str) {
    let description = schema
        .pointer(&format!("/properties/{property}/description"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing description for property {property}: {schema:#}"));
    assert!(
        description
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase()),
        "description for {property:?} should contain {expected:?}, got {description:?}"
    );
}
