//! Adapter-boundary types for host runtimes.
//!
//! These are intentionally small and runtime-agnostic: they name the host
//! integration points (Pi, Claude Code, Codex/OpenAI, OpenCode, MCP, JSONL/CLI)
//! without pulling host-specific logic into `aver-core`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::tools::AverTools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterRuntime {
    Pi,
    ClaudeCode,
    CodexOpenAi,
    OpenCode,
    Mcp,
    JsonlCliHarness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PiAdapterConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaudeCodeAdapterConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodexOpenAiAdapterConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OpenCodeAdapterConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpAdapterConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct JsonlCliAdapterConfig;

pub trait AdapterRuntimeConfig {
    fn runtime(&self) -> AdapterRuntime;
}

impl AdapterRuntimeConfig for PiAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::Pi
    }
}

impl AdapterRuntimeConfig for ClaudeCodeAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::ClaudeCode
    }
}

impl AdapterRuntimeConfig for CodexOpenAiAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::CodexOpenAi
    }
}

impl AdapterRuntimeConfig for OpenCodeAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::OpenCode
    }
}

impl AdapterRuntimeConfig for McpAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::Mcp
    }
}

impl AdapterRuntimeConfig for JsonlCliAdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterRuntime::JsonlCliHarness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "runtime", content = "config", rename_all = "snake_case")]
pub enum AdapterConfig {
    Pi(PiAdapterConfig),
    ClaudeCode(ClaudeCodeAdapterConfig),
    CodexOpenAi(CodexOpenAiAdapterConfig),
    OpenCode(OpenCodeAdapterConfig),
    Mcp(McpAdapterConfig),
    JsonlCliHarness(JsonlCliAdapterConfig),
}

impl AdapterConfig {
    pub fn from_runtime(runtime: AdapterRuntime) -> Self {
        match runtime {
            AdapterRuntime::Pi => Self::Pi(PiAdapterConfig),
            AdapterRuntime::ClaudeCode => Self::ClaudeCode(ClaudeCodeAdapterConfig),
            AdapterRuntime::CodexOpenAi => Self::CodexOpenAi(CodexOpenAiAdapterConfig),
            AdapterRuntime::OpenCode => Self::OpenCode(OpenCodeAdapterConfig),
            AdapterRuntime::Mcp => Self::Mcp(McpAdapterConfig),
            AdapterRuntime::JsonlCliHarness => Self::JsonlCliHarness(JsonlCliAdapterConfig),
        }
    }

    pub fn runtime(&self) -> AdapterRuntime {
        match self {
            Self::Pi(config) => config.runtime(),
            Self::ClaudeCode(config) => config.runtime(),
            Self::CodexOpenAi(config) => config.runtime(),
            Self::OpenCode(config) => config.runtime(),
            Self::Mcp(config) => config.runtime(),
            Self::JsonlCliHarness(config) => config.runtime(),
        }
    }
}

impl AdapterRuntimeConfig for AdapterConfig {
    fn runtime(&self) -> AdapterRuntime {
        AdapterConfig::runtime(self)
    }
}

/// Pi agent hook trait. Runtime adapters implement this to translate host-specific
/// hook calls into Aver core operations.
pub trait PiHook {
    /// Hook called when a Pi session starts. Implementations should record a session-start event.
    fn session_start(&self, session_id: &str, agent_id: Option<&str>) -> anyhow::Result<()>;

    /// Hook called when a Pi agent takes an action. Implementations should record an episodic event.
    /// Returns the recorded event ID.
    fn agent_action(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        agent_id: Option<&str>,
    ) -> anyhow::Result<i64>;

    /// Hook called when a Pi session ends or compacts. Implementations should trigger catch-up observation.
    fn session_compact(&self, session_id: &str) -> anyhow::Result<String>;
}

/// Pi adapter that translates Pi hooks into Aver core operations.
/// This implements ADR-0016's requirement for runtime adapters outside `aver-core`.
pub struct PiAdapter {
    tools: std::sync::Arc<std::sync::Mutex<AverTools>>,
}

impl PiAdapter {
    /// Create a new Pi adapter with the given tools.
    pub fn new(tools: std::sync::Arc<std::sync::Mutex<AverTools>>) -> Self {
        Self { tools }
    }
}

impl PiHook for PiAdapter {
    fn session_start(&self, session_id: &str, agent_id: Option<&str>) -> anyhow::Result<()> {
        let tools = self
            .tools
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let agent = agent_id.unwrap_or("pi");
        // Record session start event
        let event = tools.record_event(crate::tools::RecordEventParams {
            session_id: session_id.to_string(),
            kind: "session_start".to_string(),
            payload: format!("Pi agent session started for {agent}"),
            source: Some("pi-hook".to_string()),
            agent_id: Some(agent.to_string()),
            agent_kind: Some("LLM".to_string()),
            scope: Some("global".to_string()),
        })?;
        let _ = event; // Event is recorded; we only care it succeeded
        Ok(())
    }

    fn agent_action(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        agent_id: Option<&str>,
    ) -> anyhow::Result<i64> {
        let tools = self
            .tools
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let agent = agent_id.unwrap_or("pi");
        // Record agent action as episodic event
        let event = tools.record_event(crate::tools::RecordEventParams {
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            payload: payload.to_string(),
            source: Some("pi-hook".to_string()),
            agent_id: Some(agent.to_string()),
            agent_kind: Some(
                if kind == "user_message" {
                    "HUMAN"
                } else {
                    "LLM"
                }
                .to_string(),
            ),
            scope: Some("global".to_string()),
        })?;
        Ok(event.id)
    }

    fn session_compact(&self, session_id: &str) -> anyhow::Result<String> {
        let tools = self
            .tools
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        // Check coverage and assemble compaction summary
        let coverage = tools.observation_coverage(crate::tools::ObservationCoverageParams {
            session_id: session_id.to_string(),
        })?;

        if !coverage.uncovered_event_ids.is_empty() {
            return Ok(format!(
                "Compaction blocked: {} events uncovered. Run catch-up observation first.",
                coverage.uncovered_event_ids.len()
            ));
        }

        // Assemble the compaction summary
        let summary =
            tools.assemble_compaction_summary(crate::tools::AssembleCompactionSummaryParams {
                session_id: session_id.to_string(),
            })?;
        Ok(summary.summary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterRunContext {
    pub source: String,
    pub session_id: String,
    pub runtime: AdapterRuntime,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}
