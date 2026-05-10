use std::collections::BTreeSet;

use aver_server::adapters::{
    AdapterConfig, AdapterRunContext, AdapterRuntime, ClaudeCodeAdapterConfig,
    CodexOpenAiAdapterConfig, JsonlCliAdapterConfig, McpAdapterConfig, OpenCodeAdapterConfig,
    PiAdapterConfig,
};

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
