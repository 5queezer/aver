use std::path::Path;

use aver_core::{
    AgentKind, CandidateClaim, Claim, ClaimStatus, ContradictionRecord, EpisodicEvent, NewClaim,
    Observation, ObservationCoverage, ObservationRecall, ObservationRelevance, PredicateWalk,
    RecallFilters, ScopeWalk, Store,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
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
    /// ADR-0021 hierarchical memory scope. Defaults to "global".
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    pub query: String,
    #[serde(default)]
    pub alpha: Option<f64>,
    #[serde(default)]
    pub hops: Option<usize>,
    #[serde(default)]
    pub top_k: Option<usize>,
    /// ADR-0021 scope filter. Defaults to "global".
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode: "exact" | "ancestors" | "descendants" | "any".
    /// Defaults to "any" when scope is omitted, "ancestors" otherwise.
    #[serde(default)]
    pub scope_walk: Option<String>,
    /// ADR-0023 filter: only return claims written by this `agent_id`.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// ADR-0023 filter: only return claims whose `agent_kind` matches.
    /// One of HUMAN, LLM, DETERMINISTIC_PARSER, EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0023 filter: predicate (exact unless `predicate_walk=descendants`).
    #[serde(default)]
    pub predicate: Option<String>,
    /// ADR-0023 walk mode for `predicate`: "exact" | "descendants" (default exact).
    #[serde(default)]
    pub predicate_walk: Option<String>,
    /// ADR-0023 filter: inclusive lower bound on claim confidence (0..=1).
    #[serde(default)]
    pub min_confidence: Option<f64>,
    /// ADR-0023 filter: status. One of ACTIVE, SUPERSEDED, INVALIDATED, or
    /// "any" to drop the implicit ACTIVE filter. Defaults to ACTIVE.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ExpandParams {
    pub entity: String,
    #[serde(default)]
    pub hops: Option<usize>,
    #[serde(default)]
    pub predicates: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub scope_walk: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddTripleParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub source: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictNewClaimParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictParams {
    pub triple_id: i64,
    pub reason: String,
    #[serde(default)]
    pub new_claim: Option<ContradictNewClaimParams>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecordEventParams {
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ShouldExtractMemoriesParams {
    pub session_id: String,
    pub event_threshold: usize,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ProposeCandidateClaimParams {
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ListCandidateClaimsParams {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PromoteCandidateClaimParams {
    pub candidate_id: i64,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RejectCandidateClaimParams {
    pub candidate_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ObservationRelevanceParam {
    Low,
    Medium,
    High,
    Critical,
}

impl From<ObservationRelevanceParam> for ObservationRelevance {
    fn from(value: ObservationRelevanceParam) -> Self {
        match value {
            ObservationRelevanceParam::Low => Self::Low,
            ObservationRelevanceParam::Medium => Self::Medium,
            ObservationRelevanceParam::High => Self::High,
            ObservationRelevanceParam::Critical => Self::Critical,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecordObservationParams {
    pub session_id: String,
    pub content: String,
    pub relevance: ObservationRelevanceParam,
    pub source_event_ids: Vec<i64>,
    pub derivation: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ObservationCoverageParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallObservationParams {
    pub observation_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AssembleCompactionSummaryParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddVectorChunkParams {
    pub claim_id: i64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RetireClaimParams {
    pub claim_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RetireClaimView {
    pub claim_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClaimView {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub status: String,
    pub source_refs: Vec<String>,
    pub agent_id: String,
    pub agent_kind: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EventView {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: String,
    pub ts: i64,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CandidateClaimView {
    pub id: i64,
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: String,
    pub confidence: f64,
    pub status: String,
    pub promoted_claim_id: Option<i64>,
    pub rejection_reason: Option<String>,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ObservationView {
    pub id: String,
    pub session_id: String,
    pub content: String,
    pub relevance: String,
    pub source_event_ids: Vec<i64>,
    pub agent_id: String,
    pub agent_kind: String,
    pub derivation: String,
    pub ts: i64,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ObservationRecallView {
    pub observation: ObservationView,
    pub events: Vec<EventView>,
    pub audit_status: Option<String>,
    pub prune_marker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ObservationCoverageView {
    pub event_ids: Vec<i64>,
    pub covered_event_ids: Vec<i64>,
    pub uncovered_event_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CompactionSummaryView {
    pub session_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallView {
    pub triples: Vec<ClaimView>,
    pub chunks: Vec<String>,
    pub subgraph: GraphView,
    pub confidence_floor: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<String>,
    pub edges: Vec<ClaimView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AddTripleView {
    pub triple_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContradictionView {
    pub contradiction_id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConsolidateView {
    pub merged: usize,
    pub superseded: usize,
    pub decayed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShouldExtractMemoriesView {
    pub should_extract: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VectorChunkView {
    pub id: i64,
    pub claim_id: i64,
    pub text: String,
}

impl From<Claim> for ClaimView {
    fn from(claim: Claim) -> Self {
        Self {
            id: claim.id,
            subject: claim.subject,
            predicate: claim.predicate,
            object: claim.object,
            confidence: claim.confidence,
            status: claim.status.as_str().to_string(),
            source_refs: claim.source_refs,
            agent_id: claim.agent_id,
            agent_kind: claim.agent_kind.as_str().to_string(),
            scope: claim.scope,
        }
    }
}

impl From<EpisodicEvent> for EventView {
    fn from(event: EpisodicEvent) -> Self {
        Self {
            id: event.id,
            session_id: event.session_id,
            kind: event.kind,
            payload: event.payload,
            source: event.source,
            agent_id: event.agent_id,
            agent_kind: event.agent_kind.as_str().to_string(),
            ts: event.ts,
            scope: event.scope,
        }
    }
}

impl From<CandidateClaim> for CandidateClaimView {
    fn from(candidate: CandidateClaim) -> Self {
        Self {
            id: candidate.id,
            event_id: candidate.event_id,
            subject: candidate.subject,
            predicate: candidate.predicate,
            object: candidate.object,
            provenance: candidate.provenance.as_str().to_string(),
            confidence: candidate.confidence,
            status: candidate.status,
            promoted_claim_id: candidate.promoted_claim_id,
            rejection_reason: candidate.rejection_reason,
            scope: candidate.scope,
        }
    }
}

impl From<Observation> for ObservationView {
    fn from(observation: Observation) -> Self {
        Self {
            id: observation.id,
            session_id: observation.session_id,
            content: observation.content,
            relevance: observation.relevance.as_str().to_string(),
            source_event_ids: observation.source_event_ids,
            agent_id: observation.agent_id,
            agent_kind: observation.agent_kind.as_str().to_string(),
            derivation: observation.derivation,
            ts: observation.ts,
            scope: observation.scope,
        }
    }
}

impl From<ObservationRecall> for ObservationRecallView {
    fn from(recall: ObservationRecall) -> Self {
        Self {
            observation: recall.observation.into(),
            events: recall.events.into_iter().map(EventView::from).collect(),
            audit_status: recall.audit_status,
            prune_marker_id: recall.prune_marker_id,
        }
    }
}

impl From<ObservationCoverage> for ObservationCoverageView {
    fn from(coverage: ObservationCoverage) -> Self {
        Self {
            event_ids: coverage.event_ids,
            covered_event_ids: coverage.covered_event_ids,
            uncovered_event_ids: coverage.uncovered_event_ids,
        }
    }
}

impl From<ContradictionRecord> for ContradictionView {
    fn from(record: ContradictionRecord) -> Self {
        Self {
            contradiction_id: record.id,
            claim_id: record.claim_id,
            reason: record.reason,
            new_claim_id: record.new_claim_id,
            status: record.status.to_ascii_lowercase(),
        }
    }
}

pub struct AverTools {
    store: Store,
}

impl AverTools {
    pub fn open(memory_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            store: Store::open(memory_dir)?,
        })
    }

    pub fn remember_claim(&self, params: RememberClaimParams) -> anyhow::Result<ClaimView> {
        if params.subject.trim().is_empty() {
            anyhow::bail!("invalid subject: must not be empty");
        }
        if params.predicate.trim().is_empty() {
            anyhow::bail!("invalid predicate: must not be empty");
        }
        if params.object.trim().is_empty() {
            anyhow::bail!("invalid object: must not be empty");
        }
        let source = params.source.as_deref().unwrap_or("mcp");
        let agent_id = params.agent_id.as_deref().unwrap_or("mcp");
        let agent_kind = params
            .agent_kind
            .as_deref()
            .unwrap_or("EXTERNAL_TOOL")
            .parse::<AgentKind>()?;
        let scope = params.scope.as_deref().unwrap_or("global");
        let id = self.store.add_claim_from_agent_with_scope(
            agent_id,
            agent_kind,
            &params.subject,
            &params.predicate,
            &params.object,
            source,
            scope,
        )?;
        Ok(self.store.get_claim(id)?.into())
    }

    pub fn recall(&self, params: RecallParams) -> anyhow::Result<RecallView> {
        let top_k = validate_top_k(params.top_k.unwrap_or(5))?;
        let scope = params.scope.as_deref().unwrap_or("global");
        let walk = parse_scope_walk(params.scope_walk.as_deref(), &params.scope)?;
        // ADR-0023 typed filters.
        let agent_kind = match params.agent_kind.as_deref() {
            Some(s) => Some(s.parse::<AgentKind>()?),
            None => None,
        };
        let predicate_walk = match params.predicate_walk.as_deref() {
            Some(s) => s
                .parse::<PredicateWalk>()
                .map_err(|err| anyhow::anyhow!("invalid predicate_walk: {err}"))?,
            None => PredicateWalk::Exact,
        };
        let status = match params.status.as_deref() {
            Some("any") => None,
            Some(s) => Some(s.parse::<ClaimStatus>()?),
            None => Some(ClaimStatus::Active),
        };
        let filters = RecallFilters {
            scope: scope.to_string(),
            scope_walk: walk,
            agent_id: params.agent_id.clone(),
            agent_kind,
            predicate: params.predicate.clone(),
            predicate_walk,
            min_confidence: params.min_confidence,
            status,
        };
        let mut claims = self
            .store
            .recall_text_with_filters(&params.query, filters)?;
        claims.truncate(top_k);
        let _alpha = if let Some(alpha) = params.alpha {
            aver_core::retrieval::HybridWeights::try_new(alpha)
                .map_err(|err| anyhow::anyhow!("invalid alpha: {err}"))?
                .alpha
        } else {
            aver_core::retrieval::HybridWeights::for_query(&params.query).alpha
        };
        let hops = validate_hops(params.hops.unwrap_or(2))?;
        let mut subgraph = self
            .store
            .expand_with_scope(&params.query, hops, None, scope, walk)?;
        if subgraph.edges.is_empty()
            && let Some(first_claim) = claims.first()
        {
            subgraph =
                self.store
                    .expand_with_scope(&first_claim.subject, hops, None, scope, walk)?;
        }
        let confidence_floor = claims
            .iter()
            .chain(subgraph.edges.iter())
            .map(|claim| claim.confidence)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        Ok(RecallView {
            triples: claims.into_iter().map(ClaimView::from).collect(),
            chunks: Vec::new(),
            subgraph: GraphView {
                nodes: subgraph.nodes,
                edges: subgraph.edges.into_iter().map(ClaimView::from).collect(),
            },
            confidence_floor,
        })
    }

    pub fn expand(&self, params: ExpandParams) -> anyhow::Result<GraphView> {
        let hops = validate_hops(params.hops.unwrap_or(2))?;
        let predicate_refs = params
            .predicates
            .as_ref()
            .map(|items| items.iter().map(String::as_str).collect::<Vec<_>>());
        let scope = params.scope.as_deref().unwrap_or("global");
        let walk = parse_scope_walk(params.scope_walk.as_deref(), &params.scope)?;
        let graph = self.store.expand_with_scope(
            &params.entity,
            hops,
            predicate_refs.as_deref(),
            scope,
            walk,
        )?;
        Ok(GraphView {
            nodes: graph.nodes,
            edges: graph.edges.into_iter().map(ClaimView::from).collect(),
        })
    }

    pub fn add_triple(&self, params: AddTripleParams) -> anyhow::Result<AddTripleView> {
        if params.subject.trim().is_empty() {
            anyhow::bail!("invalid subject: must not be empty");
        }
        if params.predicate.trim().is_empty() {
            anyhow::bail!("invalid predicate: must not be empty");
        }
        if params.object.trim().is_empty() {
            anyhow::bail!("invalid object: must not be empty");
        }
        if let Some(confidence) = params.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            anyhow::bail!("invalid confidence: must be between 0 and 1");
        }
        let scope = params.scope.as_deref().unwrap_or("global");
        let id = self.store.add_claim_with_confidence_and_scope(
            &params.subject,
            &params.predicate,
            &params.object,
            &params.source,
            params.confidence.unwrap_or(0.95),
            scope,
        )?;
        Ok(AddTripleView {
            triple_id: id,
            status: "appended".to_string(),
        })
    }

    pub fn contradict(&self, params: ContradictParams) -> anyhow::Result<ContradictionView> {
        let new_claim = params.new_claim.as_ref().map(|claim| NewClaim {
            subject: claim.subject.as_str(),
            predicate: claim.predicate.as_str(),
            object: claim.object.as_str(),
            source: claim.source.as_str(),
        });
        Ok(self
            .store
            .contradict(params.triple_id, &params.reason, new_claim)?
            .into())
    }

    pub fn consolidate(&self, params: ConsolidateParams) -> anyhow::Result<ConsolidateView> {
        if let Some(scope) = params.scope.as_deref()
            && scope != "all"
        {
            anyhow::bail!("unsupported consolidation scope: {scope}");
        }
        let report = self.store.consolidate_report()?;
        Ok(ConsolidateView {
            merged: report.merged,
            superseded: report.superseded,
            decayed: report.decayed,
        })
    }

    pub fn record_event(&self, params: RecordEventParams) -> anyhow::Result<EventView> {
        if params.session_id.trim().is_empty() {
            anyhow::bail!("invalid session_id: must not be empty");
        }
        if params.kind.trim().is_empty() {
            anyhow::bail!("invalid kind: must not be empty");
        }
        let source = params.source.as_deref().unwrap_or("mcp");
        let agent_id = params.agent_id.as_deref().unwrap_or("mcp");
        let agent_kind = params
            .agent_kind
            .as_deref()
            .unwrap_or("EXTERNAL_TOOL")
            .parse::<AgentKind>()?;
        let scope = params.scope.as_deref().unwrap_or("global");
        let id = self.store.record_event_from_agent_with_scope(
            agent_id,
            agent_kind,
            &params.session_id,
            &params.kind,
            &params.payload,
            source,
            scope,
        )?;
        Ok(self.store.get_event(id)?.into())
    }

    pub fn should_extract_memories(
        &self,
        params: ShouldExtractMemoriesParams,
    ) -> anyhow::Result<ShouldExtractMemoriesView> {
        if params.event_threshold == 0 {
            anyhow::bail!("invalid event_threshold: must be at least 1");
        }
        Ok(ShouldExtractMemoriesView {
            should_extract: self
                .store
                .should_extract_memories(&params.session_id, params.event_threshold)?,
        })
    }

    pub fn propose_candidate_claim(
        &self,
        params: ProposeCandidateClaimParams,
    ) -> anyhow::Result<CandidateClaimView> {
        let scope = params.scope.as_deref().unwrap_or("global");
        let id = self.store.propose_candidate_claim_with_scope(
            params.event_id,
            &params.subject,
            &params.predicate,
            &params.object,
            scope,
        )?;
        Ok(self.store.get_candidate_claim(id)?.into())
    }

    pub fn list_candidate_claims(
        &self,
        params: ListCandidateClaimsParams,
    ) -> anyhow::Result<Vec<CandidateClaimView>> {
        Ok(self
            .store
            .list_candidate_claims(params.session_id.as_deref(), params.status.as_deref())?
            .into_iter()
            .map(CandidateClaimView::from)
            .collect())
    }

    pub fn promote_candidate_claim(
        &self,
        params: PromoteCandidateClaimParams,
    ) -> anyhow::Result<ClaimView> {
        let claim_id = self.store.promote_candidate_claim(params.candidate_id)?;
        Ok(self.store.get_claim(claim_id)?.into())
    }

    pub fn reject_candidate_claim(
        &self,
        params: RejectCandidateClaimParams,
    ) -> anyhow::Result<CandidateClaimView> {
        self.store
            .reject_candidate_claim(params.candidate_id, &params.reason)?;
        Ok(self.store.get_candidate_claim(params.candidate_id)?.into())
    }

    pub fn record_observation(
        &self,
        params: RecordObservationParams,
    ) -> anyhow::Result<ObservationView> {
        let scope = params.scope.as_deref().unwrap_or("global");
        let id = self.store.record_observation_with_scope(
            &params.session_id,
            &params.content,
            params.relevance.into(),
            &params.source_event_ids,
            &params.derivation,
            scope,
        )?;
        Ok(self.store.get_observation(&id)?.into())
    }

    pub fn recall_observation(
        &self,
        params: RecallObservationParams,
    ) -> anyhow::Result<ObservationRecallView> {
        Ok(self
            .store
            .recall_observation(&params.observation_id)?
            .into())
    }

    pub fn observation_coverage(
        &self,
        params: ObservationCoverageParams,
    ) -> anyhow::Result<ObservationCoverageView> {
        Ok(self.store.observation_coverage(&params.session_id)?.into())
    }

    pub fn assemble_compaction_summary(
        &self,
        params: AssembleCompactionSummaryParams,
    ) -> anyhow::Result<CompactionSummaryView> {
        Ok(CompactionSummaryView {
            summary: self.store.assemble_compaction_summary(&params.session_id)?,
            session_id: params.session_id,
        })
    }

    pub fn retire_claim(&self, params: RetireClaimParams) -> anyhow::Result<RetireClaimView> {
        if params.reason.trim().is_empty() {
            anyhow::bail!("invalid reason: must not be empty");
        }
        self.store.retire_claim(params.claim_id, &params.reason)?;
        Ok(RetireClaimView {
            claim_id: params.claim_id,
            status: "INVALIDATED".to_string(),
        })
    }

    pub fn add_vector_chunk(
        &self,
        params: AddVectorChunkParams,
    ) -> anyhow::Result<VectorChunkView> {
        if params.text.trim().is_empty() {
            anyhow::bail!("invalid text: must not be empty");
        }
        let chunk_id =
            self.store
                .add_vector_chunk(params.claim_id, &params.text, "nomic-embed-text")?;
        Ok(VectorChunkView {
            id: chunk_id,
            claim_id: params.claim_id,
            text: params.text,
        })
    }
}

/// Resolve `ScopeWalk` from optional MCP request strings.
///
/// Layer-1 default: when `scope_walk` is omitted, use `Any` if `scope` is also
/// omitted (preserves today's behavior verbatim) and `Ancestors` if a scope
/// was supplied (the user clearly wants scoped reads). Layer 2 will refine
/// this further when connection-resolved scope lands.
fn parse_scope_walk(walk: Option<&str>, scope: &Option<String>) -> anyhow::Result<ScopeWalk> {
    match walk {
        Some(s) => s
            .parse::<ScopeWalk>()
            .map_err(|err| anyhow::anyhow!("invalid scope_walk: {err}")),
        None => Ok(if scope.is_some() {
            ScopeWalk::Ancestors
        } else {
            ScopeWalk::Any
        }),
    }
}

fn validate_hops(hops: usize) -> anyhow::Result<usize> {
    if (1..=8).contains(&hops) {
        Ok(hops)
    } else {
        anyhow::bail!("invalid hops: must be between 1 and 8")
    }
}

fn validate_top_k(top_k: usize) -> anyhow::Result<usize> {
    if (1..=100).contains(&top_k) {
        Ok(top_k)
    } else {
        anyhow::bail!("invalid top_k: must be between 1 and 100")
    }
}
