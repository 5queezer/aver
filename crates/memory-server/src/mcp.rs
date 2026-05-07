use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::tools::{
    AmlTools, RecallParams as CoreRecallParams, RememberClaimParams as CoreRememberClaimParams,
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RememberClaimParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    5
}

pub struct AmlMcpService {
    tools: Arc<Mutex<AmlTools>>,
    base_url: String,
    #[allow(dead_code)]
    tool_router: ToolRouter<AmlMcpService>,
}

#[tool_router]
impl AmlMcpService {
    pub fn open(memory_dir: impl AsRef<Path>, base_url: String) -> anyhow::Result<Self> {
        Ok(Self {
            tools: Arc::new(Mutex::new(AmlTools::open(memory_dir)?)),
            base_url,
            tool_router: Self::tool_router(),
        })
    }

    #[tool(description = "Store a durable structured memory claim in AML.")]
    async fn remember_claim(
        &self,
        Parameters(params): Parameters<RememberClaimParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .tools
            .lock()
            .map_err(|err| {
                McpError::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("memory tool lock poisoned: {err}"),
                    None,
                )
            })?
            .remember_claim(CoreRememberClaimParams {
                subject: params.subject,
                predicate: params.predicate,
                object: params.object,
                source: params.source,
                agent_id: params.agent_id,
                agent_kind: params.agent_kind,
            });
        match result {
            Ok(claim) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&claim).unwrap_or_default(),
            )])),
            Err(err) => Err(McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("remember_claim failed: {err}"),
                None,
            )),
        }
    }

    #[tool(description = "Recall durable AML claims by text query.")]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .tools
            .lock()
            .map_err(|err| {
                McpError::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("memory tool lock poisoned: {err}"),
                    None,
                )
            })?
            .recall(CoreRecallParams {
                query: params.query,
                top_k: Some(params.top_k),
            });
        match result {
            Ok(claims) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&claims).unwrap_or_default(),
            )])),
            Err(err) => Err(McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("recall failed: {err}"),
                None,
            )),
        }
    }
}

#[tool_handler]
impl ServerHandler for AmlMcpService {
    fn get_info(&self) -> ServerInfo {
        let icon_url = format!("{}/icon.svg", self.base_url);
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("agent-memory-layer", env!("CARGO_PKG_VERSION"))
                    .with_title("AML Memory Server")
                    .with_description("Structured claim memory server for agents.")
                    .with_icons(vec![Icon::new(icon_url).with_mime_type("image/svg+xml")]),
            )
            .with_instructions(
                "Available tools: remember_claim (store a structured claim), recall (search durable claims).".to_string(),
            )
    }
}
