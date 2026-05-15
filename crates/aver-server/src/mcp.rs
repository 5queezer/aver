use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, schemars, service::RequestContext, tool,
    tool_handler, tool_router,
};
use serde::Deserialize;

use crate::http::GrantedScopes;
use crate::scope_resolution::ResolvedScope;
use crate::scopes::{ALL_TOOL_NAMES, Scope, required_scope_for_tool};
use crate::tools::{
    AddTripleParams, AddVectorChunkParams, AssembleCompactionSummaryParams, AverTools,
    ConsolidateParams, ContradictParams, ExpandParams, ListCandidateClaimsParams,
    ObservationCoverageParams, PromoteCandidateClaimParams, ProposeCandidateClaimParams,
    RecallObservationParams, RecallParams as CoreRecallParams, RecordEventParams,
    RecordObservationParams, RejectCandidateClaimParams,
    RememberClaimParams as CoreRememberClaimParams, RetireClaimParams, ShouldExtractMemoriesParams,
};

/// Looks up the scope required for `tool_name` and verifies the request's
/// granted scopes (carried via `http::request::Parts` in `ctx.extensions`)
/// include it. Returns an `INVALID_PARAMS`-coded `insufficient_scope` error
/// per the ADR-0015 unsupported-scope contract when the check fails.
///
/// If the tool is not in the catalog, fails closed with `INTERNAL_ERROR` —
/// reaching that branch indicates a programming bug (every implemented tool
/// should be mapped).
fn require_scope(ctx: &RequestContext<RoleServer>, tool_name: &str) -> Result<(), McpError> {
    let granted: Vec<Scope> = ctx
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<GrantedScopes>())
        .map(|g| g.0.clone())
        .unwrap_or_default();
    check_scope(tool_name, &granted)
}

/// Pure scope-check used by [`require_scope`] and exercised in unit tests.
/// Returns `Ok` iff `granted` includes the scope mapped to `tool_name`.
pub(crate) fn check_scope(tool_name: &str, granted: &[Scope]) -> Result<(), McpError> {
    let required = match required_scope_for_tool(tool_name) {
        Some(s) => s,
        None => {
            return Err(McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("tool {tool_name:?} has no scope mapping"),
                None,
            ));
        }
    };
    if granted.contains(&required) {
        Ok(())
    } else {
        Err(McpError::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "insufficient_scope: tool {tool_name:?} requires scope {}",
                required.as_str()
            ),
            None,
        ))
    }
}

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
    /// ADR-0021 hierarchical memory scope. Per-call override; if omitted, the
    /// request's resolved scope (ADR-0022) applies.
    #[serde(default)]
    pub scope: Option<String>,
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
    /// ADR-0021 scope filter. Per-call override.
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode: "exact" | "ancestors" | "descendants" | "any".
    #[serde(default)]
    pub scope_walk: Option<String>,
}

fn default_top_k() -> usize {
    5
}

/// ADR-0022: read the request's resolved scope from the rmcp request
/// extensions. Precedence is: per-call param, header, default header, env,
/// then `global`. Falls back to global resolution if no middleware ran (e.g.
/// unit tests that bypass HTTP).
fn request_scope(ctx: &RequestContext<RoleServer>) -> ResolvedScope {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<ResolvedScope>())
        .cloned()
        .unwrap_or(ResolvedScope {
            scope: "global".to_string(),
            default_walk: aver_core::ScopeWalk::Any,
        })
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

    #[tool(
        description = "Store a durable claim immediately when the fact is explicit and ready for long-term memory."
    )]
    async fn remember_claim(
        &self,
        Parameters(params): Parameters<RememberClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "remember_claim")?;
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
                scope: params.scope.or_else(|| Some(request_scope(&ctx).scope)),
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

    #[tool(
        description = "Write a durable `(subject, predicate, object)` triple with provenance and optional confidence."
    )]
    async fn add_triple(
        &self,
        Parameters(mut params): Parameters<AddTripleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "add_triple")?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
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

    #[tool(
        description = "Traverse the claim graph around one entity when you already know the anchor node to inspect."
    )]
    async fn expand(
        &self,
        Parameters(mut params): Parameters<ExpandParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "expand")?;
        let resolved = request_scope(&ctx);
        if params.scope_walk.is_none() && params.scope.is_none() {
            params.scope_walk = Some(resolved.default_walk.as_str().to_string());
        }
        if params.scope.is_none() {
            params.scope = Some(resolved.scope);
        }
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

    #[tool(
        description = "Attach contradictory evidence to an existing claim without removing it from normal recall results."
    )]
    async fn contradict(
        &self,
        Parameters(params): Parameters<ContradictParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "contradict")?;
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

    #[tool(
        description = "Recompute derived claim state after writes, contradictions, or retirements when you need the latest merged view."
    )]
    async fn consolidate(
        &self,
        Parameters(params): Parameters<ConsolidateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "consolidate")?;
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

    #[tool(
        description = "Append a raw session event for later extraction; use this before candidate-claim or observation workflows."
    )]
    async fn record_event(
        &self,
        Parameters(mut params): Parameters<RecordEventParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "record_event")?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
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

    #[tool(
        description = "Check whether a session has accumulated enough events to justify extraction work."
    )]
    async fn should_extract_memories(
        &self,
        Parameters(params): Parameters<ShouldExtractMemoriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "should_extract_memories")?;
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

    #[tool(
        description = "Stage a proposed durable claim from an event before promoting it into long-term memory."
    )]
    async fn propose_candidate_claim(
        &self,
        Parameters(mut params): Parameters<ProposeCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "propose_candidate_claim")?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
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
        description = "List staged candidate claims so you can review, promote, or reject them by session and status."
    )]
    async fn list_candidate_claims(
        &self,
        Parameters(params): Parameters<ListCandidateClaimsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "list_candidate_claims")?;
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

    #[tool(description = "Promote one staged candidate claim into durable claim memory.")]
    async fn promote_candidate_claim(
        &self,
        Parameters(params): Parameters<PromoteCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "promote_candidate_claim")?;
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

    #[tool(
        description = "Reject a staged candidate claim and record why it should not become durable memory."
    )]
    async fn reject_candidate_claim(
        &self,
        Parameters(params): Parameters<RejectCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "reject_candidate_claim")?;
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

    #[tool(description = "Record a derived observation backed by one or more source events.")]
    async fn record_observation(
        &self,
        Parameters(mut params): Parameters<RecordObservationParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "record_observation")?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
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
        description = "Fetch one observation plus the exact supporting event ids when you already have an observation id."
    )]
    async fn recall_observation(
        &self,
        Parameters(params): Parameters<RecallObservationParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "recall_observation")?;
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

    #[tool(
        description = "Show which session events already have observation coverage and which still need summarization."
    )]
    async fn observation_coverage(
        &self,
        Parameters(params): Parameters<ObservationCoverageParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "observation_coverage")?;
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
            .observation_coverage(params);
        json_tool_result(result, "observation_coverage")
    }

    #[tool(
        description = "Assemble a deterministic compaction summary from current observations for session handoff or compression."
    )]
    async fn assemble_compaction_summary(
        &self,
        Parameters(params): Parameters<AssembleCompactionSummaryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "assemble_compaction_summary")?;
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

    #[tool(
        description = "Attach retrieval text to an existing claim so it participates in vector and hybrid recall."
    )]
    async fn add_vector_chunk(
        &self,
        Parameters(params): Parameters<AddVectorChunkParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "add_vector_chunk")?;
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

    #[tool(
        description = "Mark a claim as INVALIDATED so default `recall` queries no longer surface it. \
        For evidentiary contradictions that should remain in active reads pending consolidation, \
        use `contradict` instead — it does NOT change the claim's status."
    )]
    async fn retire_claim(
        &self,
        Parameters(params): Parameters<RetireClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "retire_claim")?;
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
            .retire_claim(params);
        json_tool_result(result, "retire_claim")
    }

    #[tool(
        description = "Search durable claims by text query; start here when you need stored facts but do not know an exact entity id."
    )]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, "recall")?;
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
            .recall({
                let resolved = request_scope(&ctx);
                let walk = params.scope_walk.clone().or_else(|| {
                    if params.scope.is_none() {
                        Some(resolved.default_walk.as_str().to_string())
                    } else {
                        None
                    }
                });
                CoreRecallParams {
                    query: params.query,
                    alpha: params.alpha,
                    hops: params.hops,
                    top_k: Some(params.top_k),
                    scope: params.scope.or(Some(resolved.scope)),
                    scope_walk: walk,
                    agent_id: None,
                    agent_kind: None,
                    predicate: None,
                    predicate_walk: None,
                    min_confidence: None,
                    status: None,
                }
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

fn mcp_tool_instructions() -> String {
    {
        let primary_tools = [
            "recall",
            "remember_claim",
            "record_event",
            "record_observation",
            "assemble_compaction_summary",
        ];
        let event_tools = [
            "record_event",
            "should_extract_memories",
            "propose_candidate_claim",
            "list_candidate_claims",
            "promote_candidate_claim",
            "reject_candidate_claim",
        ];
        let observation_tools = [
            "record_observation",
            "recall_observation",
            "observation_coverage",
            "assemble_compaction_summary",
        ];
        let specialized_tools = ["expand", "add_triple"];
        let maintenance_tools = [
            "contradict",
            "retire_claim",
            "consolidate",
            "add_vector_chunk",
        ];

        format!(
            concat!(
                "Aver exposes {} MCP tools. ",
                "Primary tools: {}. ",
                "Decision policy: start with recall when you need existing durable memory; use remember_claim only for explicit long-term facts; use record_event for raw session history before extraction; use record_observation and assemble_compaction_summary for source-backed handoff state. ",
                "Default workflows: recall existing memory before answering or updating; record_event -> should_extract_memories -> propose_candidate_claim/list_candidate_claims -> promote_candidate_claim or reject_candidate_claim for event-to-claim promotion; record_observation -> recall_observation or observation_coverage -> assemble_compaction_summary for continuity and compaction. ",
                "Specialized tools: {}. Use expand only when you already know the anchor entity to traverse; use add_triple only when you need explicit confidence/source control instead of the simpler remember_claim path. ",
                "Avoid routine use of maintenance tools: {}. Prefer contradict for normal conflicting evidence, retire_claim only for explicit invalidation, consolidate only when you need refreshed derived state, and add_vector_chunk only for retrieval tuning. ",
                "Tool groups: event workflow {}. Observation workflow {}. Full tool index: {}.",
                "Use event-to-claim workflow tools only when capturing session history or reviewing staged memories: {}. ",
                "Use observation continuity tools for source-backed session summaries and handoff state: {}. ",
                "Use advanced claim-maintenance tools sparingly for contradiction handling, invalidation, consolidation, and retrieval tuning: {}."
            ),
            ALL_TOOL_NAMES.len(),
            primary_tools.join(", "),
            specialized_tools.join(", "),
            maintenance_tools.join(", "),
            event_tools.join(", "),
            observation_tools.join(", "),
            ALL_TOOL_NAMES.join(", "),
            event_tools.join(", "),
            observation_tools.join(", "),
            maintenance_tools.join(", "),
        )
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
            .with_instructions(mcp_tool_instructions())
    }
}
