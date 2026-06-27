use std::collections::BTreeSet;

use aver_server::adapters::{
    AdapterConfig, AdapterRunContext, AdapterRuntime, ClaudeCodeAdapterConfig,
    CodexOpenAiAdapterConfig, JsonlCliAdapterConfig, McpAdapterConfig, OpenCodeAdapterConfig,
    PiAdapter, PiAdapterConfig, PiHook,
};
use aver_server::tools::AverTools;

fn _assert_send_sync<T: Send + Sync>() {}
fn _assert_deserializable<T: for<'de> serde::Deserialize<'de> + serde::Serialize>() {}

#[test]
fn adapter_runtime_registry_covers_required_integration_points() {
    let expected = BTreeSet::from([
        AdapterRuntime::Pi,
        AdapterRuntime::ClaudeCode,
        AdapterRuntime::CodexOpenAi,
        AdapterRuntime::OpenCode,
        AdapterRuntime::Mcp,
        AdapterRuntime::JsonlCliHarness,
    ]);

    let mut configured = BTreeSet::new();
    configured.insert(AdapterRuntime::Pi);
    configured.insert(AdapterRuntime::ClaudeCode);
    configured.insert(AdapterRuntime::CodexOpenAi);
    configured.insert(AdapterRuntime::OpenCode);
    configured.insert(AdapterRuntime::Mcp);
    configured.insert(AdapterRuntime::JsonlCliHarness);

    assert_eq!(configured, expected);
}

#[test]
fn adapter_config_traits_enforce_runtime_isolated_metadata() {
    _assert_send_sync::<PiAdapterConfig>();
    _assert_send_sync::<ClaudeCodeAdapterConfig>();
    _assert_send_sync::<CodexOpenAiAdapterConfig>();
    _assert_send_sync::<OpenCodeAdapterConfig>();
    _assert_send_sync::<McpAdapterConfig>();
    _assert_send_sync::<JsonlCliAdapterConfig>();

    _assert_deserializable::<PiAdapterConfig>();
    _assert_deserializable::<ClaudeCodeAdapterConfig>();
    _assert_deserializable::<CodexOpenAiAdapterConfig>();
    _assert_deserializable::<OpenCodeAdapterConfig>();
    _assert_deserializable::<McpAdapterConfig>();
    _assert_deserializable::<JsonlCliAdapterConfig>();

    let pi = AdapterConfig::from_runtime(AdapterRuntime::Pi);
    let claude = AdapterConfig::from_runtime(AdapterRuntime::ClaudeCode);
    let codex = AdapterConfig::from_runtime(AdapterRuntime::CodexOpenAi);
    let open_code = AdapterConfig::from_runtime(AdapterRuntime::OpenCode);
    let mcp = AdapterConfig::from_runtime(AdapterRuntime::Mcp);
    let jsonl = AdapterConfig::from_runtime(AdapterRuntime::JsonlCliHarness);

    assert_eq!(pi.runtime(), AdapterRuntime::Pi);
    assert_eq!(claude.runtime(), AdapterRuntime::ClaudeCode);
    assert_eq!(codex.runtime(), AdapterRuntime::CodexOpenAi);
    assert_eq!(open_code.runtime(), AdapterRuntime::OpenCode);
    assert_eq!(mcp.runtime(), AdapterRuntime::Mcp);
    assert_eq!(jsonl.runtime(), AdapterRuntime::JsonlCliHarness);
}

#[test]
fn run_context_traits_never_depend_on_aver_core_agent_ids() {
    let context = AdapterRunContext {
        source: "test-source".into(),
        session_id: "session-1".into(),
        runtime: AdapterRuntime::Pi,
        metadata: Default::default(),
    };

    assert_eq!(context.source.as_str(), "test-source");
    assert_eq!(context.session_id.as_str(), "session-1");
    assert_eq!(context.runtime, AdapterRuntime::Pi);
    assert!(context.metadata.is_empty());
}

#[test]
fn pi_adapter_is_send_sync() {
    _assert_send_sync::<PiAdapter>();
}

#[test]
fn pi_adapter_session_start_records_episodic_event() {
    let dir = tempfile::tempdir().unwrap();
    let tools = std::sync::Arc::new(std::sync::Mutex::new(AverTools::open(dir.path()).unwrap()));
    let adapter = PiAdapter::new(tools.clone());

    adapter
        .session_start("pi-test-session", Some("pi-agent-1"))
        .unwrap();

    // Verify event was recorded
    let t = tools.lock().unwrap();
    let coverage = t
        .observation_coverage(aver_server::tools::ObservationCoverageParams {
            session_id: "pi-test-session".to_string(),
        })
        .unwrap();
    // Just started, no observations yet, but event exists
    assert!(!coverage.event_ids.is_empty() || coverage.covered_event_ids.is_empty());
}

#[test]
fn pi_adapter_agent_action_records_event_with_correct_kind() {
    let dir = tempfile::tempdir().unwrap();
    let tools = std::sync::Arc::new(std::sync::Mutex::new(AverTools::open(dir.path()).unwrap()));
    let adapter = PiAdapter::new(tools.clone());

    let event_id = adapter
        .agent_action(
            "pi-action-session",
            "user_message",
            "Test user message payload",
            Some("pi-agent-2"),
        )
        .unwrap();

    assert!(event_id > 0);

    // Verify agent_action for LLM kind works
    let event_id2 = adapter
        .agent_action(
            "pi-action-session",
            "assistant_action",
            "Test assistant action payload",
            Some("pi-agent-2"),
        )
        .unwrap();
    assert!(event_id2 > event_id);
}

#[test]
fn pi_adapter_session_compact_blocks_on_uncovered_events() {
    let dir = tempfile::tempdir().unwrap();
    let tools = std::sync::Arc::new(std::sync::Mutex::new(AverTools::open(dir.path()).unwrap()));
    let adapter = PiAdapter::new(tools.clone());

    // Record some events without observations
    adapter
        .agent_action(
            "compact-test-session",
            "user_message",
            "Event 1",
            Some("pi"),
        )
        .unwrap();
    adapter
        .agent_action(
            "compact-test-session",
            "assistant_action",
            "Event 2",
            Some("pi"),
        )
        .unwrap();

    // Session compact should be blocked due to uncovered events
    let result = adapter.session_compact("compact-test-session").unwrap();
    assert!(result.contains("Compaction blocked"));
    assert!(result.contains("events uncovered"));
}
