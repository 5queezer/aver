//! Adapter-boundary types for host runtimes.
//!
//! These are intentionally small and runtime-agnostic: they name the host
//! integration points (Pi, Claude Code, Codex/OpenAI, OpenCode, MCP, JSONL/CLI)
//! without pulling host-specific logic into `aver-core`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterRunContext {
    pub source: String,
    pub session_id: String,
    pub runtime: AdapterRuntime,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}
