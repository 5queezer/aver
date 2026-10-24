use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, schemars, service::RequestContext, tool,
    tool_handler, tool_router,
};
use serde::Deserialize;

use crate::http::{AuthenticatedClient, GrantedScopes};
use crate::scope_resolution::ResolvedScope;
use crate::scopes::{Scope, ToolScope};
use crate::tools::{
    AddTripleParams, AddVectorChunkParams, AssembleCompactionSummaryParams, AverTools,
    ConsolidateParams, ContradictParams, ExpandParams, ListCandidateClaimsParams,
    ObservationCoverageParams, PromoteCandidateClaimParams, ProposeCandidateClaimParams,
    RecallObservationParams, RecallParams as CoreRecallParams, RecordEventParams,
    RecordObservationParams, RejectCandidateClaimParams,
    RememberClaimParams as CoreRememberClaimParams, RetireClaimParams, ShouldExtractMemoriesParams,
    TriageCandidateClaimParams,
};

/// Verifies the request's granted scopes (carried via `http::request::Parts`
/// in `ctx.extensions`) include the scope `tool` declares. Returns an
/// `INVALID_PARAMS`-coded `insufficient_scope` error per the ADR-0015
/// unsupported-scope contract when the check fails.
fn require_scope(ctx: &RequestContext<RoleServer>, tool: ToolScope) -> Result<(), McpError> {
    let granted = ctx
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<GrantedScopes>());
    if granted.is_some_and(|g| g.0.contains(&tool.scope)) {
        Ok(())
    } else {
        Err(McpError::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "insufficient_scope: tool {:?} requires scope {}",
                tool.name,
                tool.scope.as_str()
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
    /// Agent identifier to attribute the claim to. Defaults to the
    /// authenticated OAuth client id.
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

/// `agent_id` stamped on MCP writes when no authenticated client is known.
const UNAUTHENTICATED_AGENT_ID: &str = "mcp";

/// Default `agent_id` for writes: the OAuth `client_id` the bearer token was
/// issued to (stable, non-secret, `aver-<uuid>` for registered clients). An
/// explicit `agent_id` tool parameter still takes precedence. Falls back to
/// [`UNAUTHENTICATED_AGENT_ID`] only when the token carries no client (tokens
/// minted outside the OAuth code flow) or no HTTP middleware ran.
fn request_agent_id(ctx: &RequestContext<RoleServer>) -> String {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<AuthenticatedClient>())
        .map(|client| client.0.clone())
        .unwrap_or_else(|| UNAUTHENTICATED_AGENT_ID.to_string())
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
        Ok(Self::from_shared_tools(
            Arc::new(Mutex::new(AverTools::open(memory_dir)?)),
            base_url,
        ))
    }

    /// Builds a service facade over an already-open, shared [`AverTools`].
    /// The HTTP server opens one store at startup and hands a clone of the
    /// same `Arc` to every MCP session, preserving the single-writer
    /// invariant on the underlying SQLite connection.
    pub fn from_shared_tools(tools: Arc<Mutex<AverTools>>, base_url: String) -> Self {
        Self {
            tools,
            base_url,
            tool_router: Self::tool_router(),
        }
    }

    fn lock_tools(&self) -> Result<MutexGuard<'_, AverTools>, McpError> {
        self.tools.lock().map_err(|err| {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("memory tool lock poisoned: {err}"),
                None,
            )
        })
    }

    /// ADR-0020: every MCP tool with the OAuth scope it requires. Each entry
    /// is the constant declared directly above that tool's handler, which
    /// passes it to [`require_scope`]. The order is the tool index rendered
    /// in the server instructions.
    pub const TOOLS: &[ToolScope] = &[
        Self::RECALL,
        Self::EXPAND,
        Self::REMEMBER_CLAIM,
        Self::ADD_TRIPLE,
        Self::CONTRADICT,
        Self::CONSOLIDATE,
        Self::ADD_VECTOR_CHUNK,
        Self::RECORD_EVENT,
        Self::SHOULD_EXTRACT_MEMORIES,
        Self::PROPOSE_CANDIDATE_CLAIM,
        Self::LIST_CANDIDATE_CLAIMS,
        Self::PROMOTE_CANDIDATE_CLAIM,
        Self::TRIAGE_CANDIDATE_CLAIM,
        Self::REJECT_CANDIDATE_CLAIM,
        Self::RECALL_OBSERVATION,
        Self::OBSERVATION_COVERAGE,
        Self::ASSEMBLE_COMPACTION_SUMMARY,
        Self::RECORD_OBSERVATION,
        Self::RETIRE_CLAIM,
    ];

    const REMEMBER_CLAIM: ToolScope = ToolScope::new("remember_claim", Scope::ClaimsWrite);
    #[tool(
        description = "Store one explicit, stable long-term fact as a durable claim. Prefer record_event when durability is uncertain; never store secrets or transient chat."
    )]
    async fn remember_claim(
        &self,
        Parameters(params): Parameters<RememberClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::REMEMBER_CLAIM)?;
        let tools = self.lock_tools()?;
        let result = tools.remember_claim(CoreRememberClaimParams {
            subject: params.subject,
            predicate: params.predicate,
            object: params.object,
            source: params.source,
            agent_id: params.agent_id.or_else(|| Some(request_agent_id(&ctx))),
            agent_kind: params.agent_kind,
            scope: params.scope.or_else(|| Some(request_scope(&ctx).scope)),
        });
        json_tool_result(&tools, result, "remember_claim")
    }

    const ADD_TRIPLE: ToolScope = ToolScope::new("add_triple", Scope::ClaimsWrite);
    #[tool(
        description = "Write a durable `(subject, predicate, object)` triple with required provenance and optional confidence. Prefer remember_claim unless you need explicit source/confidence control."
    )]
    async fn add_triple(
        &self,
        Parameters(mut params): Parameters<AddTripleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::ADD_TRIPLE)?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
        let tools = self.lock_tools()?;
        let result = tools.add_triple(params);
        json_tool_result(&tools, result, "add_triple")
    }

    const EXPAND: ToolScope = ToolScope::new("expand", Scope::ClaimsRead);
    #[tool(
        description = "Traverse the claim graph around a known anchor entity. Use after recall identifies an entity; do not use for broad text search."
    )]
    async fn expand(
        &self,
        Parameters(mut params): Parameters<ExpandParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::EXPAND)?;
        let resolved = request_scope(&ctx);
        if params.scope_walk.is_none() && params.scope.is_none() {
            params.scope_walk = Some(resolved.default_walk.as_str().to_string());
        }
        if params.scope.is_none() {
            params.scope = Some(resolved.scope);
        }
        let tools = self.lock_tools()?;
        let result = tools.expand(params);
        json_tool_result(&tools, result, "expand")
    }

    const CONTRADICT: ToolScope = ToolScope::new("contradict", Scope::ClaimsWrite);
    #[tool(
        description = "Attach contradictory evidence to an existing claim while keeping it visible to recall. Use retire_claim for explicit invalidation."
    )]
    async fn contradict(
        &self,
        Parameters(params): Parameters<ContradictParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::CONTRADICT)?;
        let tools = self.lock_tools()?;
        let result = tools.contradict(params);
        json_tool_result(&tools, result, "contradict")
    }

    const CONSOLIDATE: ToolScope = ToolScope::new("consolidate", Scope::ClaimsWrite);
    #[tool(
        description = "Recompute derived claim state after writes, contradictions, or retirements. Use only when you need a refreshed merged view."
    )]
    async fn consolidate(
        &self,
        Parameters(params): Parameters<ConsolidateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::CONSOLIDATE)?;
        let tools = self.lock_tools()?;
        let result = tools.consolidate(params);
        json_tool_result(&tools, result, "consolidate")
    }

    const RECORD_EVENT: ToolScope = ToolScope::new("record_event", Scope::EventsWrite);
    #[tool(
        description = "Append raw session history for later extraction. Use for useful context that is not yet a durable fact; prefer remember_claim for explicit stable facts."
    )]
    async fn record_event(
        &self,
        Parameters(mut params): Parameters<RecordEventParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::RECORD_EVENT)?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
        if params.agent_id.is_none() {
            params.agent_id = Some(request_agent_id(&ctx));
        }
        let tools = self.lock_tools()?;
        let result = tools.record_event(params);
        json_tool_result(&tools, result, "record_event")
    }

    const SHOULD_EXTRACT_MEMORIES: ToolScope =
        ToolScope::new("should_extract_memories", Scope::EventsWrite);
    #[tool(
        description = "Check whether a session has enough recorded events to justify memory extraction. Use before staging event-derived candidate claims."
    )]
    async fn should_extract_memories(
        &self,
        Parameters(params): Parameters<ShouldExtractMemoriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::SHOULD_EXTRACT_MEMORIES)?;
        let tools = self.lock_tools()?;
        let result = tools.should_extract_memories(params);
        json_tool_result(&tools, result, "should_extract_memories")
    }

    const PROPOSE_CANDIDATE_CLAIM: ToolScope =
        ToolScope::new("propose_candidate_claim", Scope::CandidatesManage);
    #[tool(
        description = "Stage a proposed durable claim from a recorded event. Use in the event-to-claim workflow before human/agent review and promotion."
    )]
    async fn propose_candidate_claim(
        &self,
        Parameters(mut params): Parameters<ProposeCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::PROPOSE_CANDIDATE_CLAIM)?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
        let tools = self.lock_tools()?;
        let result = tools.propose_candidate_claim(params);
        json_tool_result(&tools, result, "propose_candidate_claim")
    }

    const LIST_CANDIDATE_CLAIMS: ToolScope =
        ToolScope::new("list_candidate_claims", Scope::CandidatesManage);
    #[tool(
        description = "List staged candidate claims by session/status for review. Use before promote_candidate_claim or reject_candidate_claim."
    )]
    async fn list_candidate_claims(
        &self,
        Parameters(params): Parameters<ListCandidateClaimsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::LIST_CANDIDATE_CLAIMS)?;
        let tools = self.lock_tools()?;
        let result = tools.list_candidate_claims(params);
        json_tool_result(&tools, result, "list_candidate_claims")
    }

    const PROMOTE_CANDIDATE_CLAIM: ToolScope =
        ToolScope::new("promote_candidate_claim", Scope::CandidatesManage);
    #[tool(
        description = "Promote one reviewed staged candidate into durable claim memory. Use only after the candidate has been inspected."
    )]
    async fn promote_candidate_claim(
        &self,
        Parameters(params): Parameters<PromoteCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::PROMOTE_CANDIDATE_CLAIM)?;
        let tools = self.lock_tools()?;
        let result = tools.promote_candidate_claim(params);
        json_tool_result(&tools, result, "promote_candidate_claim")
    }

    const TRIAGE_CANDIDATE_CLAIM: ToolScope =
        ToolScope::new("triage_candidate_claim", Scope::CandidatesManage);
    #[tool(
        description = "Let the local decision model promote, reject, or hold one staged candidate claim; promoted claims carry INFERRED provenance with confidence 0.45, and the tool errors unless AVER_DECISION_MODEL is set."
    )]
    async fn triage_candidate_claim(
        &self,
        Parameters(params): Parameters<TriageCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::TRIAGE_CANDIDATE_CLAIM)?;
        let tools = self.lock_tools()?;
        let result = tools.triage_candidate_claim(params);
        json_tool_result(&tools, result, "triage_candidate_claim")
    }

    const REJECT_CANDIDATE_CLAIM: ToolScope =
        ToolScope::new("reject_candidate_claim", Scope::CandidatesManage);
    #[tool(
        description = "Reject a staged candidate claim and record why it should not become durable memory. Use instead of silently dropping bad candidates."
    )]
    async fn reject_candidate_claim(
        &self,
        Parameters(params): Parameters<RejectCandidateClaimParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::REJECT_CANDIDATE_CLAIM)?;
        let tools = self.lock_tools()?;
        let result = tools.reject_candidate_claim(params);
        json_tool_result(&tools, result, "reject_candidate_claim")
    }

    const RECORD_OBSERVATION: ToolScope =
        ToolScope::new("record_observation", Scope::ObservationsWrite);
    #[tool(
        description = "Record a source-backed session observation for continuity, handoff, or compaction. Use source event ids for auditability."
    )]
    async fn record_observation(
        &self,
        Parameters(mut params): Parameters<RecordObservationParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::RECORD_OBSERVATION)?;
        if params.scope.is_none() {
            params.scope = Some(request_scope(&ctx).scope);
        }
        let tools = self.lock_tools()?;
        let result = tools.record_observation(params);
        json_tool_result(&tools, result, "record_observation")
    }

    const RECALL_OBSERVATION: ToolScope =
        ToolScope::new("recall_observation", Scope::ObservationsRead);
    #[tool(
        description = "Fetch one observation plus its supporting events when you already have an observation id. Use for exact provenance, not semantic search."
    )]
    async fn recall_observation(
        &self,
        Parameters(params): Parameters<RecallObservationParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::RECALL_OBSERVATION)?;
        let tools = self.lock_tools()?;
        let result = tools.recall_observation(params);
        json_tool_result(&tools, result, "recall_observation")
    }

    const OBSERVATION_COVERAGE: ToolScope =
        ToolScope::new("observation_coverage", Scope::ObservationsRead);
    #[tool(
        description = "Show which session events have observation coverage and which still need summarization. Use before compaction/handoff cleanup."
    )]
    async fn observation_coverage(
        &self,
        Parameters(params): Parameters<ObservationCoverageParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::OBSERVATION_COVERAGE)?;
        let tools = self.lock_tools()?;
        let result = tools.observation_coverage(params);
        json_tool_result(&tools, result, "observation_coverage")
    }

    const ASSEMBLE_COMPACTION_SUMMARY: ToolScope =
        ToolScope::new("assemble_compaction_summary", Scope::ObservationsRead);
    #[tool(
        description = "Assemble a deterministic handoff/compaction summary from recorded observations. Use after source-backed observations exist."
    )]
    async fn assemble_compaction_summary(
        &self,
        Parameters(params): Parameters<AssembleCompactionSummaryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::ASSEMBLE_COMPACTION_SUMMARY)?;
        let tools = self.lock_tools()?;
        let result = tools.assemble_compaction_summary(params);
        json_tool_result(&tools, result, "assemble_compaction_summary")
    }

    const ADD_VECTOR_CHUNK: ToolScope = ToolScope::new("add_vector_chunk", Scope::ClaimsWrite);
    #[tool(
        description = "Attach extra retrieval text to an existing claim for vector/hybrid recall tuning. Use sparingly when normal claim text is insufficient."
    )]
    async fn add_vector_chunk(
        &self,
        Parameters(params): Parameters<AddVectorChunkParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::ADD_VECTOR_CHUNK)?;
        let tools = self.lock_tools()?;
        let result = tools.add_vector_chunk(params);
        json_tool_result(&tools, result, "add_vector_chunk")
    }

    const RETIRE_CLAIM: ToolScope = ToolScope::new("retire_claim", Scope::ClaimsWrite);
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
        require_scope(&ctx, Self::RETIRE_CLAIM)?;
        let tools = self.lock_tools()?;
        let result = tools.retire_claim(params);
        json_tool_result(&tools, result, "retire_claim")
    }

    const RECALL: ToolScope = ToolScope::new("recall", Scope::ClaimsRead);
    #[tool(
        description = "Search durable claims by text query. Start here for stored facts, preferences, project context, or when you do not know an exact entity id."
    )]
    async fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        require_scope(&ctx, Self::RECALL)?;
        let tools = self.lock_tools()?;
        let result = tools.recall({
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
        json_tool_result(&tools, result, "recall")
    }
}

fn json_tool_result<T: serde::Serialize>(
    tools: &AverTools,
    result: anyhow::Result<T>,
    tool_name: &str,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Err(McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("{tool_name} result serialization failed: {err}"),
                None,
            )),
        },
        Err(err) => Err(McpError::new(
            ErrorCode::INTERNAL_ERROR,
            format!("{tool_name} failed: {}", tools.describe_error(&err)),
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
            "should_extract_memories",
            "propose_candidate_claim",
            "list_candidate_claims",
            "promote_candidate_claim",
            "reject_candidate_claim",
        ];
        let observation_tools = ["recall_observation", "observation_coverage"];
        let specialized_tools = ["expand", "add_triple"];
        let maintenance_tools = [
            "contradict",
            "retire_claim",
            "consolidate",
            "add_vector_chunk",
        ];

        format!(
            concat!(
                "Aver exposes {} MCP tools, but most sessions should start with the default active set. ",
                "Default active set: {}. ",
                "Decision policy: recall before answering when durable memory may matter; remember_claim only for explicit stable long-term facts; record_event for useful raw history; record_observation and assemble_compaction_summary for source-backed continuity. ",
                "Self-healing writes: predicates are strict, but unknown-predicate errors include fuzzy/semantic suggestions, accepted aliases, and a retry hint; retry with a suggested predicate rather than inventing new predicates or silently rewriting claims. Use `relates_to` as the safest generic fallback when no precise predicate fits. ",
                "Scope policy: pass `scope` only for an explicit per-call override; otherwise the request's resolved scope applies. For scoped reads, use `scope_walk=ancestors` for normal project memory, `exact` for isolation, `descendants` for subtree audits, and `any` only for cross-scope search. ",
                "Proactive memory policy: record durable user-shared preferences, project facts, and long-lived working context even without a direct remember command; only record identity details when they are necessary, user-shared, and not sensitive personal data; prefer record_event over remember_claim when durability is uncertain. Avoid storing secrets, credentials, sensitive personal data, transient chat, or facts you cannot explain with provenance. ",
                "Default workflows: recall existing memory before answering or updating; record_event -> should_extract_memories -> propose_candidate_claim/list_candidate_claims -> promote_candidate_claim or reject_candidate_claim for event-to-claim promotion; record_observation -> recall_observation or observation_coverage -> assemble_compaction_summary for continuity and compaction. ",
                "Graph navigation tools available: {}. Progressively load graph navigation tools only after recall returns an entity or you already know an anchor: expand; use add_triple instead of remember_claim only when confidence/source control is required. ",
                "Keep maintenance tools hidden until there is an explicit repair need: {}. Prefer contradict for normal conflicting evidence, retire_claim only for explicit invalidation, consolidate only when you need refreshed derived state, and add_vector_chunk only for retrieval tuning. ",
                "Tool groups: event workflow {}. Observation workflow {}. Full tool index: {}.",
                "Progressively load event-to-claim tools only when converting raw events into reviewed durable claims: {}. ",
                "Progressively load observation audit tools only for handoff, compaction, or provenance checks: {}. ",
                "Use advanced claim-maintenance tools sparingly for contradiction handling, invalidation, consolidation, and retrieval tuning: {}."
            ),
            AverMcpService::TOOLS.len(),
            primary_tools.join(", "),
            specialized_tools.join(", "),
            maintenance_tools.join(", "),
            event_tools.join(", "),
            observation_tools.join(", "),
            AverMcpService::TOOLS
                .iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
                .join(", "),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn declared_tool_scopes_match_router_and_pre_refactor_mapping() {
        // Tool -> scope mapping as `scopes::required_scope_for_tool` encoded it
        // before tools declared their own scope. Behaviour must not drift.
        let fixture: BTreeMap<&str, Scope> = [
            ("recall", Scope::ClaimsRead),
            ("expand", Scope::ClaimsRead),
            ("remember_claim", Scope::ClaimsWrite),
            ("add_triple", Scope::ClaimsWrite),
            ("contradict", Scope::ClaimsWrite),
            ("consolidate", Scope::ClaimsWrite),
            ("add_vector_chunk", Scope::ClaimsWrite),
            ("retire_claim", Scope::ClaimsWrite),
            ("record_event", Scope::EventsWrite),
            ("should_extract_memories", Scope::EventsWrite),
            ("propose_candidate_claim", Scope::CandidatesManage),
            ("list_candidate_claims", Scope::CandidatesManage),
            ("promote_candidate_claim", Scope::CandidatesManage),
            ("triage_candidate_claim", Scope::CandidatesManage),
            ("reject_candidate_claim", Scope::CandidatesManage),
            ("recall_observation", Scope::ObservationsRead),
            ("observation_coverage", Scope::ObservationsRead),
            ("assemble_compaction_summary", Scope::ObservationsRead),
            ("record_observation", Scope::ObservationsWrite),
        ]
        .into_iter()
        .collect();

        let declared: BTreeMap<&str, Scope> = AverMcpService::TOOLS
            .iter()
            .map(|tool| (tool.name, tool.scope))
            .collect();
        assert_eq!(
            declared.len(),
            AverMcpService::TOOLS.len(),
            "duplicate tool declaration"
        );
        assert_eq!(declared, fixture);

        // Every routed tool has a declaration and vice versa: the router is
        // the source of which tools exist, not a hand-kept name list.
        let routed: BTreeSet<String> = AverMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let declared_names: BTreeSet<String> =
            declared.keys().map(|name| name.to_string()).collect();
        assert_eq!(routed, declared_names);
    }

    #[test]
    fn services_sharing_tools_observe_each_others_writes() {
        // Two service facades (one per MCP session) over one shared
        // `Arc<Mutex<AverTools>>` write into the same store — the
        // single-writer invariant the HTTP layer relies on.
        let dir = tempfile::tempdir().unwrap();
        let shared = Arc::new(Mutex::new(AverTools::open(dir.path()).unwrap()));
        let service_a =
            AverMcpService::from_shared_tools(shared.clone(), "http://localhost:3317".to_string());
        let service_b =
            AverMcpService::from_shared_tools(shared.clone(), "http://localhost:3317".to_string());

        std::thread::scope(|scope| {
            let a = &service_a;
            let b = &service_b;
            let writer_a = scope.spawn(move || {
                a.lock_tools()
                    .unwrap()
                    .remember_claim(crate::tools::RememberClaimParams {
                        subject: "writer-a".to_string(),
                        predicate: "relates_to".to_string(),
                        object: "shared".to_string(),
                        source: None,
                        agent_id: None,
                        agent_kind: None,
                        scope: None,
                    })
                    .unwrap();
            });
            let writer_b = scope.spawn(move || {
                b.lock_tools()
                    .unwrap()
                    .remember_claim(crate::tools::RememberClaimParams {
                        subject: "writer-b".to_string(),
                        predicate: "relates_to".to_string(),
                        object: "shared".to_string(),
                        source: None,
                        agent_id: None,
                        agent_kind: None,
                        scope: None,
                    })
                    .unwrap();
            });
            writer_a.join().unwrap();
            writer_b.join().unwrap();
        });

        let tools = shared.lock().unwrap();
        let recalled = tools
            .recall(crate::tools::RecallParams {
                query: "shared".to_string(),
                hops: None,
                top_k: Some(10),
                scope: None,
                scope_walk: None,
                agent_id: None,
                agent_kind: None,
                predicate: None,
                predicate_walk: None,
                min_confidence: None,
                status: None,
            })
            .unwrap();
        let subjects: Vec<&str> = recalled
            .triples
            .iter()
            .map(|claim| claim.subject.as_str())
            .collect();
        assert!(
            subjects.contains(&"writer-a") && subjects.contains(&"writer-b"),
            "both facades' writes land in the shared store: {subjects:?}",
        );
    }

    #[test]
    fn json_tool_result_enriches_unknown_predicate_for_any_tool_name() {
        let dir = tempfile::tempdir().unwrap();
        let tools = AverTools::open(dir.path()).unwrap();

        let err = json_tool_result::<serde_json::Value>(
            &tools,
            Err(aver_core::Error::UnknownPredicate {
                name: "requirse".to_string(),
            }
            .into()),
            "add_triple",
        )
        .expect_err("unknown predicate should become an MCP error");

        let msg = err.message.to_string();
        assert!(
            msg.contains("add_triple failed: unknown predicate: requirse"),
            "{msg}"
        );
        assert!(
            msg.contains("did you mean `requires` (alias for `depends_on`)?"),
            "{msg}"
        );
        assert!(msg.contains("available predicates:"), "{msg}");
        assert!(msg.contains("accepted aliases:"), "{msg}");
    }
}
