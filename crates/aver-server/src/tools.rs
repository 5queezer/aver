use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use aver_core::candidate_decision::CandidateTriage;
use aver_core::decision::{DecisionClient, DecisionConfig, client_from_env};
use aver_core::retrieval::HybridWeights;
use aver_core::vector::{EmbeddingClient, EmbeddingError, VectorIndexConfig};
use aver_core::{
    AgentKind, CandidateClaim, Claim, ClaimStatus, ContradictionRecord, EpisodicEvent, Error,
    ExtractionTriggerReason, NewClaim, Observation, ObservationCoverage, ObservationRecall,
    ObservationRelevance, PredicateWalk, RecallFilters, ScopeWalk, Store,
};
use aver_extractor::LANGUAGES;
use aver_extractor::bridge::extract_tree;
use serde::{Deserialize, Serialize};

use crate::recall_log::{RecallMissFilters, RecallMissLog, RecallRequestShape};

/// Query-embedding client for hybrid recall (ADR-0004), built at the server
/// edge. `None` keeps recall on the keyword path.
pub type Embedder = Box<dyn EmbeddingClient + Send + Sync>;

/// How many hybrid candidates core ranks before the server applies the
/// ADR-0021/0023 filters and truncates to `top_k`. Core's vector half asks
/// the `vec0` index for `4 * top_k` neighbours and sqlite-vec caps `k` at
/// 4096, so 1024 is the largest pool that stays on the indexed path.
const HYBRID_CANDIDATE_POOL: usize = 1024;

/// Reads the embedding configuration once. `AVER_EMBEDDING_URL` (an Ollama
/// base URL) opts in; `AVER_EMBEDDING_MODEL` overrides the
/// [`VectorIndexConfig`] default model. Returns the client (if enabled) and
/// the model label recorded on vector chunks.
pub fn embedding_from_env() -> (Option<Embedder>, String) {
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let model =
        VectorIndexConfig::from_optional_config(None, var("AVER_EMBEDDING_MODEL").as_deref())
            .unwrap_or_default()
            .embedding_model;
    (ollama_embedder(var("AVER_EMBEDDING_URL"), &model), model)
}

#[cfg(feature = "ollama")]
fn ollama_embedder(url: Option<String>, model: &str) -> Option<Embedder> {
    url.map(|url| Box::new(aver_core::vector::OllamaEmbeddingClient::new(url, model)) as Embedder)
}

#[cfg(not(feature = "ollama"))]
fn ollama_embedder(_url: Option<String>, _model: &str) -> Option<Embedder> {
    None
}

/// Hands core the query embedding the server already computed, so one recall
/// embeds the query once even though claims and chunks are ranked separately.
struct PrecomputedEmbedding<'a>(&'a [f32]);

impl EmbeddingClient for PrecomputedEmbedding<'_> {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(self.0.to_vec())
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RememberClaimParams {
    /// Durable memory subject.
    pub subject: String,
    /// Durable memory predicate.
    pub predicate: String,
    /// Durable memory object.
    pub object: String,
    /// Source provenance for the claim; include where the fact came from so future agents can audit it.
    #[serde(default)]
    pub source: Option<String>,
    /// Agent identifier to attribute the claim to. Defaults to the authenticated OAuth client id when used through MCP.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Agent kind for attribution. One of HUMAN, LLM, DETERMINISTIC_PARSER, EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0021 hierarchical memory scope. Defaults to the caller's resolved scope when used through MCP.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    /// Natural-language or entity query to retrieve durable memory. Start broad enough to find related claims, then narrow with filters if needed.
    pub query: String,
    /// Graph hop limit.
    #[serde(default)]
    pub hops: Option<usize>,
    /// Maximum results to return. Defaults to 5 in the MCP wrapper when omitted.
    #[serde(default)]
    pub top_k: Option<usize>,
    /// ADR-0021 scope filter. Defaults to "global".
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode: "exact" | "ancestors" | "descendants" | "any".
    /// Use exact for one scope only, ancestors to include parent scopes, descendants for child scopes, and any for broad recall.
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
    /// ADR-0004 hybrid weight on the vector score (0..=1; the graph score
    /// gets `1 - alpha`). Defaults to the query-intent classifier.
    #[serde(default)]
    pub alpha: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ExpandParams {
    /// Anchor entity to traverse from.
    pub entity: String,
    /// Hop limit for graph traversal.
    #[serde(default)]
    pub hops: Option<usize>,
    /// Predicate allowlist for traversal.
    #[serde(default)]
    pub predicates: Option<Vec<String>>,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode.
    #[serde(default)]
    pub scope_walk: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CommunitiesParams {
    /// ADR-0021 scope filter applied to community members.
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode.
    #[serde(default)]
    pub scope_walk: Option<String>,
    /// Drop communities with fewer in-scope members. Defaults to 2.
    #[serde(default)]
    pub min_members: Option<usize>,
    /// Maximum communities to return, highest score first. Defaults to 20, at most 100.
    #[serde(default)]
    pub top_k: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PathParams {
    /// Entity the path starts from.
    pub from: String,
    /// Entity the path ends at.
    pub to: String,
    /// Hop limit. Defaults to 4, at most 8.
    #[serde(default)]
    pub max_hops: Option<usize>,
    /// Predicate allowlist for traversal (same semantics as `expand`).
    #[serde(default)]
    pub predicates: Option<Vec<String>>,
    /// Inclusive lower bound on each edge's confidence (0..=1).
    #[serde(default)]
    pub min_confidence: Option<f64>,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0021 walk mode.
    #[serde(default)]
    pub scope_walk: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddTripleParams {
    /// Subject of the triple.
    pub subject: String,
    /// Predicate of the triple.
    pub predicate: String,
    /// Object of the triple.
    pub object: String,
    /// Optional confidence override.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Required source reference.
    pub source: String,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictNewClaimParams {
    /// Replacement claim subject.
    pub subject: String,
    /// Replacement claim predicate.
    pub predicate: String,
    /// Replacement claim object.
    pub object: String,
    /// Source for the replacement claim.
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictParams {
    /// Claim/triple identifier to contradict.
    pub triple_id: i64,
    /// Human-readable contradiction reason.
    pub reason: String,
    /// Optional replacement claim.
    #[serde(default)]
    pub new_claim: Option<ContradictNewClaimParams>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    /// Consolidation scope. Only `"all"` (or omission) is accepted today —
    /// consolidation always recomputes derived state across every scope;
    /// per-scope consolidation is not implemented.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecordEventParams {
    /// Session identifier for the event stream.
    pub session_id: String,
    /// Stable event label such as user_message, assistant_action, tool_result, or decision.
    pub kind: String,
    /// Raw session content to preserve for later extraction; use this for context that is not yet a durable claim.
    pub payload: String,
    /// Optional event source.
    #[serde(default)]
    pub source: Option<String>,
    /// Agent identifier to attribute the event to. Defaults to the authenticated OAuth client id when used through MCP.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Agent kind for attribution.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ShouldExtractMemoriesParams {
    /// Session identifier to inspect.
    pub session_id: String,
    /// Event count threshold for extraction.
    pub event_threshold: usize,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ProposeCandidateClaimParams {
    /// Source event identifier.
    pub event_id: i64,
    /// Proposed claim subject.
    pub subject: String,
    /// Proposed claim predicate.
    pub predicate: String,
    /// Proposed claim object.
    pub object: String,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ListCandidateClaimsParams {
    /// Optional session filter.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Candidate status filter.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PromoteCandidateClaimParams {
    /// Candidate claim identifier.
    pub candidate_id: i64,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TriageCandidateClaimParams {
    /// PENDING candidate claim identifier to triage.
    pub candidate_id: i64,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RejectCandidateClaimParams {
    /// Candidate claim identifier.
    pub candidate_id: i64,
    /// Rejection reason.
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
    /// Session identifier for the observation.
    pub session_id: String,
    /// Observation content.
    pub content: String,
    /// Observation relevance tier.
    pub relevance: ObservationRelevanceParam,
    /// Source event identifiers backing the observation.
    pub source_event_ids: Vec<i64>,
    /// Derivation note or prompt fragment.
    pub derivation: String,
    /// ADR-0021 scope filter.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ObservationCoverageParams {
    /// Session identifier to inspect.
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallObservationParams {
    /// Observation identifier.
    pub observation_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AssembleCompactionSummaryParams {
    /// Session identifier to summarize.
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddVectorChunkParams {
    /// Claim identifier to attach the vector chunk to.
    pub claim_id: i64,
    /// Raw text to embed.
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RetireClaimParams {
    /// Claim identifier to retire.
    pub claim_id: i64,
    /// Retirement reason.
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
    /// Which retrieval ran: "hybrid" (ADR-0004 vector + graph blend) or
    /// "keyword" (text recall; no embedding client, no embedded chunks, a
    /// non-ACTIVE status filter, or the embedding call failed).
    pub retrieval: String,
    /// The ADR-0004 alpha used; present only when `retrieval` is "hybrid".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<String>,
    pub edges: Vec<ClaimView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommunityView {
    pub id: String,
    pub score: f64,
    pub members: Vec<String>,
    pub bridges: Vec<String>,
    pub size: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommunitiesView {
    pub communities: Vec<CommunityView>,
    pub scope: String,
    pub scope_walk: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PathStepView {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub claim_id: i64,
    pub confidence: f64,
    pub provenance: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PathView {
    pub found: bool,
    pub hops: usize,
    /// Product of the step confidences; 0 when no path was found.
    pub confidence: f64,
    pub path: Vec<PathStepView>,
    pub scope: String,
    pub scope_walk: String,
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
    /// ADR-0024 trigger reasons; present only when a decision model is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasons: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TriageCandidateClaimView {
    pub candidate_id: i64,
    /// One of "promote", "reject", "hold".
    pub triage: String,
    /// Durable claim id when the candidate was promoted.
    pub claim_id: Option<i64>,
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

/// Default `AVER_EXTRACT_MAX_FILES`.
const DEFAULT_EXTRACT_MAX_FILES: usize = 2000;

/// Bounds for the `extract` tool (ADR-0027): the only directory tree it may
/// read, and the most files one call may scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractLimits {
    /// `AVER_EXTRACT_ROOT`. `None` makes the tool refuse every path.
    pub root: Option<PathBuf>,
    /// `AVER_EXTRACT_MAX_FILES`.
    pub max_files: usize,
}

impl ExtractLimits {
    pub fn from_env() -> Self {
        Self {
            root: std::env::var_os("AVER_EXTRACT_ROOT")
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
            max_files: std::env::var("AVER_EXTRACT_MAX_FILES")
                .ok()
                .and_then(|value| value.trim().parse().ok())
                .filter(|max| *max > 0)
                .unwrap_or(DEFAULT_EXTRACT_MAX_FILES),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractParams {
    pub path: String,
    pub scope: Option<String>,
    pub lang: Option<String>,
    pub dry_run: bool,
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractView {
    pub scope: String,
    pub files_scanned: usize,
    pub files_failed: usize,
    pub facts_extracted: usize,
    pub facts_ingested: usize,
    /// Unsupported predicates by name, plus `duplicate` and `privacy`.
    pub skipped: BTreeMap<String, usize>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub struct AverTools {
    store: Store,
    extract: ExtractLimits,
    /// ADR-0024 decision model; `None` unless `AVER_DECISION_MODEL` opts in.
    decision: Option<Box<dyn DecisionClient + Send + Sync>>,
    /// ADR-0024 confidence floor (`AVER_DECISION_MIN_CONFIDENCE`); choices
    /// below it fall back to the deterministic behaviour.
    decision_min_confidence: f64,
    /// ADR-0004 embedding client; `None` unless `AVER_EMBEDDING_URL` opts in.
    embedder: Option<Embedder>,
    /// Model label recorded on vector chunks (`AVER_EMBEDDING_MODEL`).
    embedding_model: String,
    /// Content-free recall-miss log; `None` unless `AVER_RECALL_MISS_LOG=1`.
    recall_miss_log: Option<RecallMissLog>,
}

impl AverTools {
    pub fn open(memory_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::open_with_decision_client(memory_dir, client_from_env())
    }

    /// Like [`AverTools::open`] with an explicit decision client (tests inject
    /// a mock here instead of setting `AVER_DECISION_MODEL`).
    pub fn open_with_decision_client(
        memory_dir: impl AsRef<Path>,
        decision: Option<Box<dyn DecisionClient + Send + Sync>>,
    ) -> anyhow::Result<Self> {
        let (embedder, embedding_model) = embedding_from_env();
        let recall_miss_log = RecallMissLog::from_env(memory_dir.as_ref());
        Ok(Self {
            store: Store::open(memory_dir)?,
            extract: ExtractLimits::from_env(),
            decision,
            decision_min_confidence: DecisionConfig::from_env().min_confidence,
            embedder,
            embedding_model,
            recall_miss_log,
        })
    }

    /// Replaces the recall-miss log read from the environment (tests inject
    /// a log at a temp path instead of setting `AVER_RECALL_MISS_LOG`).
    pub fn with_recall_miss_log(mut self, log: Option<RecallMissLog>) -> Self {
        self.recall_miss_log = log;
        self
    }

    /// Replaces the embedding client read from the environment (tests and
    /// the BEAM parity harness inject a deterministic client here).
    pub fn with_embedding_client(mut self, embedder: Option<Embedder>) -> Self {
        self.embedder = embedder;
        self
    }

    /// Replaces the `extract` limits read from the environment (tests inject
    /// a root here instead of setting `AVER_EXTRACT_ROOT`).
    pub fn with_extract_limits(mut self, extract: ExtractLimits) -> Self {
        self.extract = extract;
        self
    }

    /// Agent-facing text for a tool error. Core validation errors are the
    /// single enforcement point for input rules; where core's `Display`
    /// does not name the MCP parameter or the rule, it is rephrased here.
    pub fn describe_error(&self, err: &anyhow::Error) -> String {
        let Some(core_err) = err.downcast_ref::<Error>() else {
            return err.to_string();
        };
        match core_err {
            Error::UnknownPredicate { name } => self
                .store
                .describe_unknown_predicate(name)
                .unwrap_or_else(|vocab_err| {
                    format!(
                        "unknown predicate: {name} (not in predicate_types or predicate_alias). failed to load available predicates: {vocab_err}"
                    )
                }),
            Error::InvalidClaimField { field }
            | Error::InvalidEventField { field }
            | Error::InvalidObservationField { field } => {
                format!("invalid {field}: must not be empty")
            }
            Error::InvalidConfidence { value } => {
                format!("invalid confidence: {value}; must be between 0 and 1")
            }
            Error::InvalidEventThreshold => {
                "invalid event_threshold: must be greater than zero".to_string()
            }
            Error::InvalidContradictionReason | Error::InvalidRejectionReason => {
                "invalid reason: must not be empty".to_string()
            }
            Error::InvalidVectorChunkText => "invalid text: must not be empty".to_string(),
            Error::InvalidGraphEntity => "invalid entity: must not be empty".to_string(),
            Error::InvalidGraphHops => "invalid hops: must be greater than zero".to_string(),
            Error::InvalidPredicateFilter => {
                "invalid predicates: entries must not be empty".to_string()
            }
            Error::InvalidRecallQuery => {
                "invalid query: must contain at least one searchable term".to_string()
            }
            Error::InvalidAgentId { value } => format!(
                "invalid agent_id: {value:?}; must be non-empty and match [A-Za-z0-9_-]"
            ),
            other => other.to_string(),
        }
    }

    pub fn remember_claim(&self, params: RememberClaimParams) -> anyhow::Result<ClaimView> {
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

    /// ADR-0027 `extract` tool: the server-side twin of `aver extract`.
    /// The path must be absolute and, once canonicalized (symlinks resolved),
    /// inside `AVER_EXTRACT_ROOT`; the bridge walk does not follow symlinks.
    pub fn extract(&self, params: ExtractParams) -> anyhow::Result<ExtractView> {
        let Some(root) = &self.extract.root else {
            anyhow::bail!(
                "extract is disabled: the server operator must set AVER_EXTRACT_ROOT to the directory extract may read"
            );
        };
        if let Some(lang) = params.lang.as_deref()
            && !LANGUAGES.contains(&lang)
        {
            anyhow::bail!(
                "invalid lang: {lang}; expected one of {}",
                LANGUAGES.join(", ")
            );
        }
        if !Path::new(&params.path).is_absolute() {
            anyhow::bail!("invalid path: must be absolute");
        }
        let root = root.canonicalize().map_err(|err| {
            anyhow::anyhow!("AVER_EXTRACT_ROOT {} is unusable: {err}", root.display())
        })?;
        let path = Path::new(&params.path)
            .canonicalize()
            .map_err(|err| anyhow::anyhow!("invalid path: {}: {err}", params.path))?;
        if !path.starts_with(&root) {
            anyhow::bail!("invalid path: must be inside AVER_EXTRACT_ROOT");
        }
        let scope = params.scope.unwrap_or_else(|| "global".to_string());
        let extraction = extract_tree(&path, params.lang.as_deref())?;
        if extraction.files_scanned() > self.extract.max_files {
            anyhow::bail!(
                "too many files: {} scanned, AVER_EXTRACT_MAX_FILES is {}; extract a narrower path",
                extraction.files_scanned(),
                self.extract.max_files
            );
        }

        let mut skipped = extraction.skipped.clone();
        skipped.insert("duplicate".to_string(), extraction.duplicates);
        let mut ingested = 0;
        let mut privacy = 0;
        if !params.dry_run {
            for file in &extraction.files {
                for fact in &file.facts {
                    match self.store.add_claim_from_agent_with_scope(
                        &params.agent_id,
                        AgentKind::DeterministicParser,
                        &fact.subject,
                        &fact.predicate,
                        &fact.object,
                        &file.path,
                        &scope,
                    ) {
                        Ok(_) => ingested += 1,
                        Err(Error::Privacy(_)) => privacy += 1,
                        Err(err) => return Err(err.into()),
                    }
                }
            }
            // No consolidate here: its conflict rule supersedes every claim
            // with a newer one of the same subject and predicate, which
            // retires multi-valued code facts (`File:x defines A` vs `B`).
            // Same as the CLI; see ADR-0027.
        }
        skipped.insert("privacy".to_string(), privacy);
        let note = (scope == "global").then(|| {
            "project scoping was not applied: facts were written to the global scope".to_string()
        });
        Ok(ExtractView {
            scope,
            files_scanned: extraction.files_scanned(),
            files_failed: extraction.failed.len(),
            facts_extracted: extraction.extracted,
            facts_ingested: ingested,
            skipped,
            dry_run: params.dry_run,
            note,
        })
    }

    /// [`AverTools::recall`] for an MCP caller: when the recall-miss log is
    /// enabled, a miss or weak hit is recorded against `agent_id` (the
    /// authenticated client id). Retrieval is unchanged; logging never fails
    /// the recall.
    pub fn recall_for_agent(
        &self,
        params: RecallParams,
        agent_id: &str,
    ) -> anyhow::Result<RecallView> {
        let Some(log) = &self.recall_miss_log else {
            return self.recall(params);
        };
        let started = std::time::Instant::now();
        let shape = recall_request_shape(&params);
        let view = self.recall(params)?;
        log.observe(shape, &view, agent_id, started.elapsed());
        Ok(view)
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
        if let Some(min) = filters.min_confidence
            && !(0.0..=1.0).contains(&min)
        {
            return Err(Error::InvalidConfidence { value: min }.into());
        }
        let weights = match params.alpha {
            Some(alpha) => HybridWeights::try_new(alpha)
                .map_err(|err| anyhow::anyhow!("invalid alpha: {err}"))?,
            None => HybridWeights::for_query(&params.query),
        };
        let (mut claims, chunks, retrieval, alpha) =
            match self.recall_hybrid(&params.query, &filters, top_k, weights)? {
                Some((claims, chunks)) => (claims, chunks, "hybrid", Some(weights.alpha)),
                None => (
                    self.store
                        .recall_text_with_filters(&params.query, filters)?,
                    Vec::new(),
                    "keyword",
                    None,
                ),
            };
        claims.truncate(top_k);
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
            chunks,
            subgraph: GraphView {
                nodes: subgraph.nodes,
                edges: subgraph.edges.into_iter().map(ClaimView::from).collect(),
            },
            confidence_floor,
            retrieval: retrieval.to_string(),
            alpha,
        })
    }

    /// ADR-0004 hybrid half of [`AverTools::recall`]: claims ranked by core's
    /// `recall_hybrid_claims_with_alpha`, then filtered (core's hybrid recall
    /// takes no ADR-0021/0023 filters) and truncated to `top_k`, plus the
    /// vector-chunk texts of those claims ranked by similarity to the query.
    ///
    /// `Ok(None)` means "use the keyword path": no embedding client, no
    /// embedded chunks, a status filter other than ACTIVE (core's hybrid
    /// recall returns ACTIVE claims only), or the embedding call failed.
    fn recall_hybrid(
        &self,
        query: &str,
        filters: &RecallFilters,
        top_k: usize,
        weights: HybridWeights,
    ) -> anyhow::Result<Option<(Vec<Claim>, Vec<String>)>> {
        let Some(embedder) = self.embedder.as_deref() else {
            return Ok(None);
        };
        if filters.status != Some(ClaimStatus::Active)
            || self.store.vector_chunk_embedding_status()?.0 == 0
        {
            return Ok(None);
        }
        let query_embedding = match embedder.embed(query) {
            Ok(embedding) => embedding,
            Err(err) => {
                eprintln!("aver-server: query embedding failed, using keyword recall: {err}");
                return Ok(None);
            }
        };
        let mut claims = Vec::new();
        for claim in self.store.recall_hybrid_claims_with_alpha(
            query,
            &PrecomputedEmbedding(&query_embedding),
            HYBRID_CANDIDATE_POOL,
            weights,
        )? {
            if self.claim_matches_filters(&claim, filters)? {
                claims.push(claim);
                if claims.len() == top_k {
                    break;
                }
            }
        }
        let mut chunks = Vec::new();
        for claim in &claims {
            for chunk in self.store.list_vector_chunks_for_claim(claim.id)? {
                let score = chunk.embedding.as_deref().and_then(|embedding| {
                    aver_core::vector::normalized_cosine_score(&query_embedding, embedding)
                });
                chunks.push((score.unwrap_or(0.0), chunk.id, chunk.text));
            }
        }
        chunks.sort_by(|(a_score, a_id, _), (b_score, b_id, _)| {
            b_score.total_cmp(a_score).then_with(|| a_id.cmp(b_id))
        });
        chunks.truncate(top_k);
        Ok(Some((
            claims,
            chunks.into_iter().map(|(_, _, text)| text).collect(),
        )))
    }

    /// Server-side mirror of the `recall_text_with_filters` WHERE clause for
    /// hybrid results. Status is not checked: hybrid recall only runs for the
    /// ACTIVE filter and core returns ACTIVE claims only.
    fn claim_matches_filters(
        &self,
        claim: &Claim,
        filters: &RecallFilters,
    ) -> anyhow::Result<bool> {
        if !scope_matches(&claim.scope, &filters.scope, filters.scope_walk)
            || filters
                .agent_id
                .as_ref()
                .is_some_and(|id| *id != claim.agent_id)
            || filters
                .agent_kind
                .is_some_and(|kind| kind != claim.agent_kind)
            || filters
                .min_confidence
                .is_some_and(|min| claim.confidence < min)
        {
            return Ok(false);
        }
        Ok(match filters.predicate.as_deref() {
            None => true,
            Some(predicate) if claim.predicate == predicate => true,
            Some(predicate) => {
                filters.predicate_walk == PredicateWalk::Descendants
                    && !predicate.trim().is_empty()
                    && self.store.predicate_implies(&claim.predicate, predicate)?
            }
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

    /// Communities from core's `detect_communities`, which runs over the
    /// whole graph. Under a scope walk other than `any`, members (and
    /// bridges) are kept only when they touch an ACTIVE claim in scope; the
    /// grouping and score stay core's whole-graph values.
    pub fn communities(&self, params: CommunitiesParams) -> anyhow::Result<CommunitiesView> {
        let top_k = validate_top_k(params.top_k.unwrap_or(20))?;
        let min_members = params.min_members.unwrap_or(2);
        let scope = params.scope.as_deref().unwrap_or("global");
        let walk = parse_scope_walk(params.scope_walk.as_deref(), &params.scope)?;
        let mut in_scope: HashMap<String, bool> = HashMap::new();
        let mut communities = Vec::new();
        for community in self.store.detect_communities()? {
            let (mut members, mut bridges) = (community.members, community.bridge_nodes);
            if walk != ScopeWalk::Any {
                for entity in &members {
                    if !in_scope.contains_key(entity) {
                        let touches = !self
                            .store
                            .expand_with_scope(entity, 1, None, scope, walk)?
                            .edges
                            .is_empty();
                        in_scope.insert(entity.clone(), touches);
                    }
                }
                members.retain(|entity| in_scope[entity]);
                bridges.retain(|entity| in_scope[entity]);
            }
            if members.is_empty() || members.len() < min_members {
                continue;
            }
            communities.push(CommunityView {
                id: community.id,
                score: community.score,
                size: members.len(),
                members,
                bridges,
            });
        }
        communities.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        communities.truncate(top_k);
        Ok(CommunitiesView {
            communities,
            scope: scope.to_string(),
            scope_walk: walk.as_str().to_string(),
        })
    }

    /// Shortest directed claim path from `from` to `to`. Core's `graph_path`
    /// takes no scope, so the server searches the scoped subgraph from
    /// `expand_with_scope` instead, ranking like `graph_path`: fewest hops,
    /// then highest confidence product, then lexicographic entity path.
    /// Hyperedges are not traversed.
    pub fn path(&self, params: PathParams) -> anyhow::Result<PathView> {
        let max_hops = validate_hops(params.max_hops.unwrap_or(4))?;
        let min_confidence = params.min_confidence.unwrap_or(0.0);
        if !(0.0..=1.0).contains(&min_confidence) {
            return Err(Error::InvalidConfidence {
                value: min_confidence,
            }
            .into());
        }
        if params.to.trim().is_empty() {
            return Err(Error::InvalidGraphEntity.into());
        }
        let predicate_refs = params
            .predicates
            .as_ref()
            .map(|items| items.iter().map(String::as_str).collect::<Vec<_>>());
        let scope = params.scope.as_deref().unwrap_or("global");
        let walk = parse_scope_walk(params.scope_walk.as_deref(), &params.scope)?;
        let graph = self.store.expand_with_scope(
            &params.from,
            max_hops,
            predicate_refs.as_deref(),
            scope,
            walk,
        )?;
        let edges: Vec<&Claim> = graph
            .edges
            .iter()
            .filter(|claim| claim.confidence >= min_confidence)
            .collect();

        let mut best: Option<(Vec<String>, Vec<&Claim>, f64)> = None;
        if params.from == params.to {
            best = Some((vec![params.from.clone()], Vec::new(), 1.0));
        }
        let mut queue = VecDeque::from([(vec![params.from.clone()], Vec::<&Claim>::new(), 1.0)]);
        while let Some((entities, steps, confidence)) = queue.pop_front() {
            if steps.len() == max_hops
                || best
                    .as_ref()
                    .is_some_and(|(_, best_steps, _)| steps.len() >= best_steps.len())
            {
                continue;
            }
            let current = entities.last().expect("path starts with `from`");
            for &edge in edges.iter().filter(|edge| &edge.subject == current) {
                if entities.contains(&edge.object) {
                    continue;
                }
                let mut next_entities = entities.clone();
                next_entities.push(edge.object.clone());
                let mut next_steps = steps.clone();
                next_steps.push(edge);
                let next_confidence = confidence * edge.confidence;
                if edge.object == params.to {
                    let replace = best.as_ref().is_none_or(|(best_entities, _, best_conf)| {
                        next_confidence.total_cmp(best_conf).is_gt()
                            || (next_confidence == *best_conf && next_entities < *best_entities)
                    });
                    if replace {
                        best = Some((next_entities, next_steps, next_confidence));
                    }
                } else {
                    queue.push_back((next_entities, next_steps, next_confidence));
                }
            }
        }

        let (found, confidence, steps) = match best {
            Some((_, steps, confidence)) => (true, confidence, steps),
            None => (false, 0.0, Vec::new()),
        };
        Ok(PathView {
            found,
            hops: steps.len(),
            confidence,
            path: steps
                .into_iter()
                .map(|claim| PathStepView {
                    subject: claim.subject.clone(),
                    predicate: claim.predicate.clone(),
                    object: claim.object.clone(),
                    claim_id: claim.id,
                    confidence: claim.confidence,
                    provenance: claim.provenance.as_str().to_string(),
                    scope: claim.scope.clone(),
                })
                .collect(),
            scope: scope.to_string(),
            scope_walk: walk.as_str().to_string(),
        })
    }

    pub fn add_triple(&self, params: AddTripleParams) -> anyhow::Result<AddTripleView> {
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
        let Some(client) = self.decision.as_deref() else {
            return Ok(ShouldExtractMemoriesView {
                should_extract: self
                    .store
                    .should_extract_memories(&params.session_id, params.event_threshold)?,
                reasons: None,
            });
        };
        let decision = self.store.extraction_decision_with_client(
            &params.session_id,
            params.event_threshold,
            None,
            client,
            self.decision_min_confidence,
        )?;
        // Same rule as `Store::should_extract_memories`: a coverage gap alone
        // does not justify extraction.
        Ok(ShouldExtractMemoriesView {
            should_extract: decision
                .reasons
                .iter()
                .any(|reason| *reason != ExtractionTriggerReason::UncoveredCoverageGap),
            reasons: Some(
                decision
                    .reasons
                    .iter()
                    .map(|reason| reason.as_str().to_string())
                    .collect(),
            ),
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

    pub fn triage_candidate_claim(
        &self,
        params: TriageCandidateClaimParams,
    ) -> anyhow::Result<TriageCandidateClaimView> {
        let client = self.decision.as_deref().ok_or_else(|| {
            anyhow::anyhow!("decision model not configured (set AVER_DECISION_MODEL)")
        })?;
        let (triage, claim_id) = match self.store.triage_candidate_claim(
            params.candidate_id,
            client,
            self.decision_min_confidence,
        )? {
            CandidateTriage::Promote => (
                "promote",
                self.store
                    .get_candidate_claim(params.candidate_id)?
                    .promoted_claim_id,
            ),
            CandidateTriage::Reject => ("reject", None),
            CandidateTriage::Hold => ("hold", None),
        };
        Ok(TriageCandidateClaimView {
            candidate_id: params.candidate_id,
            triage: triage.to_string(),
            claim_id,
        })
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
        // Embed only text that core's validation and privacy filter would
        // accept, so rejected text never leaves the process. Without a client,
        // or when embedding fails, the chunk is stored text-only.
        let embedding = self
            .embedder
            .as_deref()
            .filter(|_| {
                !params.text.trim().is_empty() && aver_core::privacy_filter(&params.text).is_ok()
            })
            .and_then(|embedder| match embedder.embed(&params.text) {
                Ok(embedding) => Some(embedding),
                Err(err) => {
                    eprintln!("aver-server: chunk embedding failed, storing text only: {err}");
                    None
                }
            });
        let chunk_id = match embedding {
            Some(embedding) => self.store.add_vector_chunk_with_embedding(
                params.claim_id,
                &params.text,
                &self.embedding_model,
                &embedding,
            )?,
            None => {
                self.store
                    .add_vector_chunk(params.claim_id, &params.text, &self.embedding_model)?
            }
        };
        Ok(VectorChunkView {
            id: chunk_id,
            claim_id: params.claim_id,
            text: params.text,
        })
    }
}

/// ADR-0021 scope walk over one claim's scope; the same rule as core's
/// `scope_filter_sql`, applied to rows core's hybrid recall returns unfiltered.
fn scope_matches(claim_scope: &str, scope: &str, walk: ScopeWalk) -> bool {
    let is_strict_prefix = |prefix: &str, path: &str| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
    };
    match walk {
        ScopeWalk::Any => true,
        ScopeWalk::Exact => claim_scope == scope,
        ScopeWalk::Descendants => claim_scope == scope || is_strict_prefix(scope, claim_scope),
        ScopeWalk::Ancestors => {
            claim_scope == "global" || claim_scope == scope || is_strict_prefix(claim_scope, scope)
        }
    }
}

/// Resolve `ScopeWalk` from optional MCP request strings.
///
/// Layer-1 default: when `scope_walk` is omitted, use `Any` if `scope` is also
/// omitted (preserves today's behavior verbatim) and `Ancestors` if a scope
/// was supplied (the user clearly wants scoped reads). Layer 2 will refine
/// this further when connection-resolved scope lands.
/// Reduces recall params to the content-free shape the recall-miss log
/// records: the query becomes two counts, filters become flags and enum
/// names. Values `recall` would reject map to defaults; such a request
/// errors before anything is logged.
fn recall_request_shape(params: &RecallParams) -> RecallRequestShape {
    let (query_len, query_tokens) = RecallRequestShape::query_counts(&params.query);
    let status = match params.status.as_deref() {
        Some("any") => "any",
        Some(s) => s.parse::<ClaimStatus>().map_or("any", ClaimStatus::as_str),
        None => ClaimStatus::Active.as_str(),
    };
    RecallRequestShape {
        query_len,
        query_tokens,
        scope: params.scope.clone().unwrap_or_else(|| "global".to_string()),
        scope_walk: parse_scope_walk(params.scope_walk.as_deref(), &params.scope)
            .unwrap_or(ScopeWalk::Any)
            .as_str(),
        filters: RecallMissFilters {
            agent_id_set: params.agent_id.is_some(),
            agent_kind: params
                .agent_kind
                .as_deref()
                .and_then(|kind| kind.parse::<AgentKind>().ok())
                .map(AgentKind::as_str),
            predicate_set: params.predicate.is_some(),
            min_confidence: params.min_confidence,
            status,
        },
        top_k: params.top_k.unwrap_or(5),
    }
}

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

/// Server-side traversal cap. Core rejects `hops == 0`
/// (`Error::InvalidGraphHops`) but has no upper bound.
fn validate_hops(hops: usize) -> anyhow::Result<usize> {
    if hops <= 8 {
        Ok(hops)
    } else {
        anyhow::bail!("invalid hops: must be at most 8")
    }
}

/// Core's `recall_text_with_filters` takes no `top_k`; the server truncates
/// its result, so this bound has no core counterpart on this path.
fn validate_top_k(top_k: usize) -> anyhow::Result<usize> {
    if (1..=100).contains(&top_k) {
        Ok(top_k)
    } else {
        anyhow::bail!("invalid top_k: must be between 1 and 100")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matches_follows_adr_0021_walks() {
        assert!(scope_matches("proj/a", "proj/a", ScopeWalk::Exact));
        assert!(!scope_matches("proj/a/x", "proj/a", ScopeWalk::Exact));
        assert!(scope_matches("proj/a/x", "proj/a", ScopeWalk::Descendants));
        assert!(!scope_matches("proj/ab", "proj/a", ScopeWalk::Descendants));
        assert!(scope_matches("global", "proj/a/x", ScopeWalk::Ancestors));
        assert!(scope_matches("proj", "proj/a/x", ScopeWalk::Ancestors));
        assert!(!scope_matches(
            "proj/a/x/y",
            "proj/a/x",
            ScopeWalk::Ancestors
        ));
        assert!(!scope_matches("pro", "proj/a", ScopeWalk::Ancestors));
        assert!(scope_matches("anything", "proj/a", ScopeWalk::Any));
    }
}
