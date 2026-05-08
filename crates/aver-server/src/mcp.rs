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
    AddTripleParams, AddVectorChunkParams, AssembleCompactionSummaryParams, AverTools,
    ConsolidateParams, ContradictParams, ExpandParams, ListCandidateClaimsParams,
    PromoteCandidateClaimParams, ProposeCandidateClaimParams, RecallObservationParams,
    RecallParams as CoreRecallParams, RecordEventParams, RecordObservationParams,
    RejectCandidateClaimParams, RememberClaimParams as CoreRememberClaimParams,
    ShouldExtractMemoriesParams,
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
    #[serde(default)]
    pub alpha: Option<f64>,
    #[serde(default)]
    pub hops: Option<usize>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    5
}

pub struct AverMcpService {
    tools: Arc<Mutex<AverTools>>,
    base_url: String,
    #[allow(dead_code)]
    tool_router: ToolRouter<AverMcpService>,
}

#[tool_router]
impl AverMcpService {
    pub fn open(memory_dir: impl AsRef<Path>, base_url: String) -> anyhow::Result<Self> {
        Ok(Self {
            tools: Arc::new(Mutex::new(AverTools::open(memory_dir)?)),
            base_url,
            tool_router: Self::tool_router(),
        })
    }

    #[tool(description = "Store a durable structured memory claim in Aver.")]
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

    #[tool(description = "Append a structured memory triple per ADR-0008.")]
    async fn add_triple(
        &self,
        Parameters(params): Parameters<AddTripleParams>,
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
            .add_triple(params);
        json_tool_result(result, "add_triple")
    }

    #[tool(description = "Expand a known entity into its local claim-graph neighborhood.")]
    async fn expand(
        &self,
        Parameters(params): Parameters<ExpandParams>,
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
            .expand(params);
        json_tool_result(result, "expand")
    }

    #[tool(description = "Record an explicit contradiction for an existing claim.")]
    async fn contradict(
        &self,
        Parameters(params): Parameters<ContradictParams>,
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
            .contradict(params);
        json_tool_result(result, "contradict")
    }

    #[tool(description = "Run Aver's on-demand consolidation pass.")]
    async fn consolidate(
        &self,
        Parameters(params): Parameters<ConsolidateParams>,
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
            .consolidate(params);
        json_tool_result(result, "consolidate")
    }

    #[tool(description = "Record an append-only episodic event for later memory extraction.")]
    async fn record_event(
        &self,
        Parameters(params): Parameters<RecordEventParams>,
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
            .record_event(params);
        json_tool_result(result, "record_event")
    }

    #[tool(description = "Return whether a session should trigger memory extraction.")]
    async fn should_extract_memories(
        &self,
        Parameters(params): Parameters<ShouldExtractMemoriesParams>,
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
            .should_extract_memories(params);
        json_tool_result(result, "should_extract_memories")
    }

    #[tool(description = "Stage a candidate claim from episodic event provenance.")]
    async fn propose_candidate_claim(
        &self,
        Parameters(params): Parameters<ProposeCandidateClaimParams>,
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
            .propose_candidate_claim(params);
        json_tool_result(result, "propose_candidate_claim")
    }

    #[tool(
        description = "List staged candidate claims, optionally filtered by session_id and status."
    )]
    async fn list_candidate_claims(
        &self,
        Parameters(params): Parameters<ListCandidateClaimsParams>,
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
            .list_candidate_claims(params);
        json_tool_result(result, "list_candidate_claims")
    }

    #[tool(description = "Promote a staged candidate claim to durable Aver memory.")]
    async fn promote_candidate_claim(
        &self,
        Parameters(params): Parameters<PromoteCandidateClaimParams>,
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
            .promote_candidate_claim(params);
        json_tool_result(result, "promote_candidate_claim")
    }

    #[tool(description = "Reject a staged candidate claim with a reason.")]
    async fn reject_candidate_claim(
        &self,
        Parameters(params): Parameters<RejectCandidateClaimParams>,
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
            .reject_candidate_claim(params);
        json_tool_result(result, "reject_candidate_claim")
    }

    #[tool(description = "Record a source-backed episodic observation projection.")]
    async fn record_observation(
        &self,
        Parameters(params): Parameters<RecordObservationParams>,
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
            .record_observation(params);
        json_tool_result(result, "record_observation")
    }

    #[tool(
        description = "Recall an observation and its exact supporting events by observation id."
    )]
    async fn recall_observation(
        &self,
        Parameters(params): Parameters<RecallObservationParams>,
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
            .recall_observation(params);
        json_tool_result(result, "recall_observation")
    }

    #[tool(description = "Mechanically assemble a compaction summary from current observations.")]
    async fn assemble_compaction_summary(
        &self,
        Parameters(params): Parameters<AssembleCompactionSummaryParams>,
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
            .assemble_compaction_summary(params);
        json_tool_result(result, "assemble_compaction_summary")
    }

    #[tool(description = "Attach a text chunk to a claim for vector indexing.")]
    async fn add_vector_chunk(
        &self,
        Parameters(params): Parameters<AddVectorChunkParams>,
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
            .add_vector_chunk(params);
        json_tool_result(result, "add_vector_chunk")
    }

    #[tool(description = "Recall durable Aver claims by text query.")]
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
                alpha: params.alpha,
                hops: params.hops,
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

fn json_tool_result<T: serde::Serialize>(
    result: anyhow::Result<T>,
    tool_name: &str,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&value).unwrap_or_default(),
        )])),
        Err(err) => Err(McpError::new(
            ErrorCode::INTERNAL_ERROR,
            format!("{tool_name} failed: {err}"),
            None,
        )),
    }
}

#[tool_handler]
impl ServerHandler for AverMcpService {
    fn get_info(&self) -> ServerInfo {
        let icon_url = format!("{}/icon.svg", self.base_url);
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("aver", env!("CARGO_PKG_VERSION"))
                    .with_title("Aver Server")
                    .with_description("Structured claim memory server for agents.")
                    .with_icons(vec![Icon::new(icon_url).with_mime_type("image/svg+xml")]),
            )
            .with_instructions(
                "Available tools: recall, expand, add_triple, contradict, consolidate, remember_claim, record_event, should_extract_memories, propose_candidate_claim, list_candidate_claims, promote_candidate_claim, reject_candidate_claim, record_observation, recall_observation, assemble_compaction_summary, add_vector_chunk.".to_string(),
            )
    }
}
