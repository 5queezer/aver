use aver_core::{CandidateWrite, ClaimWrite, EventWrite, RecallQuery};
use std::path::Path;

use aver_core::graph_first::{GraphFirstOptions, GraphFirstRecall};
use aver_core::retrieval::{DEFAULT_RECENCY_TAU_SECS, HybridWeights};
use aver_core::{
    AgentKind, CandidateClaim, Claim, ClaimStatus, ContradictionRecord, EpisodicEvent, Error,
    PredicateWalk, RecallFilters, ReverifyWrite, ScopeWalk, Store,
};
use serde::{Deserialize, Serialize};

pub use aver_core::vector::{Embedder, embedding_from_env, embedding_from_vars};

/// Reads the opt-in recall recency term (ADR-0004) once:
/// `AVER_RECALL_RECENCY_WEIGHT` (`w` in `[0, 1]`, default `0` = off) and
/// `AVER_RECALL_RECENCY_TAU_DAYS` (positive, default 90). An invalid value
/// is reported on stderr and leaves recency off.
pub fn recency_from_env() -> (f64, f64) {
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
    };
    let off = (0.0, DEFAULT_RECENCY_TAU_SECS);
    let Some(weight) = var("AVER_RECALL_RECENCY_WEIGHT").filter(|value| !value.is_empty()) else {
        return off;
    };
    let tau_secs = match var("AVER_RECALL_RECENCY_TAU_DAYS").filter(|value| !value.is_empty()) {
        Some(days) => days.parse::<f64>().map(|days| days * 86_400.0),
        None => Ok(DEFAULT_RECENCY_TAU_SECS),
    };
    let parsed = weight
        .parse::<f64>()
        .ok()
        .zip(tau_secs.ok())
        .and_then(|(weight, tau_secs)| {
            HybridWeights::default().with_recency(weight, tau_secs).ok()
        });
    match parsed {
        Some(weights) => (weights.recency_weight, weights.recency_tau_secs),
        None => {
            eprintln!(
                "aver-server: invalid AVER_RECALL_RECENCY_WEIGHT/AVER_RECALL_RECENCY_TAU_DAYS, recency stays off"
            );
            off
        }
    }
}

/// Reads the opt-in ADR-0032 evidence ranking once:
/// `AVER_RECALL_EVIDENCE_HALF_LIFE` (closed matching items since the last
/// confirmation that halve a contextual claim's hybrid score; unset or `0` =
/// off). An invalid value is reported on stderr and leaves it off.
pub fn evidence_half_life_from_env() -> f64 {
    let Some(value) = std::env::var("AVER_RECALL_EVIDENCE_HALF_LIFE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "0")
    else {
        return 0.0;
    };
    match value
        .parse::<f64>()
        .ok()
        .and_then(|n| HybridWeights::default().with_evidence_half_life(n).ok())
    {
        Some(weights) => weights.evidence_half_life,
        None => {
            eprintln!(
                "aver-server: ignoring AVER_RECALL_EVIDENCE_HALF_LIFE={value:?}: expected a positive number of evidence items"
            );
            0.0
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RememberClaimParams {
    /// EXTRACTED = explicitly stated in source; INFERRED = your judgment (default for LLM, capped 0.75); AMBIGUOUS = conflicting evidence; USER_ASSERTED = explicit human assertion.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Object kind: entity or literal. Omitted uses deterministic classification; prose cannot be forced to entity.
    #[serde(default)]
    pub object_kind: Option<String>,
    /// Existing source event in this claim scope. Record rationale with record_event first, then cite its returned id.
    #[serde(default)]
    pub event_id: Option<i64>,
    /// Reject prose objects before appending; default accepts with an atomic-triple hint.
    #[serde(default)]
    pub strict_atomic: Option<bool>,
    /// Confidence override; otherwise use provenance policy (INFERRED defaults to 0.45).
    #[serde(default)]
    pub confidence: Option<f64>,
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
    /// Agent kind for attribution. LLM (default), HUMAN, DETERMINISTIC_PARSER, or EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0021 hierarchical memory scope. Defaults to the caller's resolved scope when used through MCP.
    #[serde(default)]
    pub scope: Option<String>,
    /// ADR-0032: the context entity this claim was established under (an opaque id such as `context:<id>`).
    #[serde(default)]
    pub context: Option<String>,
    /// ADR-0032: start of the evidence window, Unix seconds. Needs `context`.
    #[serde(default)]
    pub valid_from: Option<i64>,
    /// ADR-0032: end of the evidence window, Unix seconds. Needs `context`; reverify_claim can extend it.
    #[serde(default)]
    pub valid_until: Option<i64>,
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
    /// ADR-0032 filter: only claims established under this context entity.
    #[serde(default)]
    pub context: Option<String>,
    /// "hybrid" (default) or "graph_first" (ADR-0033: bounded expansion
    /// from entities the query names; falls back to hybrid without one).
    #[serde(default)]
    pub mode: Option<String>,
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
pub struct AddTripleParams {
    /// EXTRACTED = explicitly stated in source; INFERRED = your judgment (default for LLM, capped 0.75); AMBIGUOUS = conflicting evidence; USER_ASSERTED = explicit human assertion.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Object kind: entity or literal. Omitted uses deterministic classification; prose cannot be forced to entity.
    #[serde(default)]
    pub object_kind: Option<String>,
    /// Existing source event in this claim scope. Record rationale with record_event first, then cite its returned id.
    #[serde(default)]
    pub event_id: Option<i64>,
    /// Reject prose objects before appending; default accepts with an atomic-triple hint.
    #[serde(default)]
    pub strict_atomic: Option<bool>,

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
    /// Agent identifier to attribute the claim to. Defaults to the authenticated OAuth client id when used through MCP.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Agent kind for attribution: HUMAN, LLM (default), DETERMINISTIC_PARSER, EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0032: the context entity this claim was established under (an opaque id such as `context:<id>`).
    #[serde(default)]
    pub context: Option<String>,
    /// ADR-0032: start of the evidence window, Unix seconds. Needs `context`.
    #[serde(default)]
    pub valid_from: Option<i64>,
    /// ADR-0032: end of the evidence window, Unix seconds. Needs `context`; reverify_claim can extend it.
    #[serde(default)]
    pub valid_until: Option<i64>,
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
    /// Agent identifier to attribute the claim to. Defaults to the authenticated OAuth client id when used through MCP.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Agent kind for attribution. LLM (default), HUMAN, DETERMINISTIC_PARSER, or EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0021 scope of the replacement claim. Defaults to the contradicted claim's scope.
    #[serde(default)]
    pub scope: Option<String>,
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
pub struct GetEventParams {
    /// Source event id returned by record_event or a claim's event_id/source_refs.
    pub event_id: i64,
    /// Exact event scope; MCP defaults to the request's resolved scope. No ancestor walk.
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
pub struct RejectCandidateClaimParams {
    /// Candidate claim identifier.
    pub candidate_id: i64,
    /// Rejection reason.
    pub reason: String,
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

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ReverifyClaimParams {
    /// Claim to confirm. It must be ACTIVE and in the request scope.
    pub claim_id: i64,
    /// Evidence that confirms the claim (at least one): `event:<id>` or `claim:<id>` from the claim's scope or a scope below it, or an item id.
    pub evidence: Vec<String>,
    /// Extend the claim's window end to this Unix time. Never shrinks it; needs a claim with a context.
    #[serde(default)]
    pub valid_until: Option<i64>,
    /// Agent identifier recorded as the confirming writer. Defaults to the authenticated OAuth client id when used through MCP.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Agent kind recorded as the confirming writer: LLM (default), HUMAN, DETERMINISTIC_PARSER, EXTERNAL_TOOL.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// ADR-0021 scope of the request; it must equal the claim's scope. Defaults to the caller's resolved scope when used through MCP.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReverifyClaimView {
    pub claim_id: i64,
    pub verified_at: i64,
    pub agent_id: String,
    pub agent_kind: String,
    pub evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<i64>,
}

/// ADR-0032: the latest explicit confirmation of a claim.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfirmationView {
    pub at: i64,
    pub agent_id: String,
    pub agent_kind: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClaimView {
    pub provenance: String,
    pub object_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
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
    /// Present only on a write made with `AVER_EMBED_ON_WRITE=1` and an
    /// embedding provider: `true` when the claim got a vector chunk and is
    /// eligible for hybrid recall, `false` when embedding failed and the
    /// claim is keyword-only (nothing retries it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded: Option<bool>,
    /// ADR-0032 validity; omitted for claims without a context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<i64>,
    /// Recall only, and only for a claim with a context or an explicit
    /// confirmation: verification times, the closed matching evidence since
    /// the last confirmation (context only) and that confirmation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_confirmed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_since_confirmation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_confirmation: Option<ConfirmationView>,
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
    /// ADR-0033: present only when `mode` was "graph_first".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_first: Option<GraphFirstView>,
}

/// ADR-0033 graph-first details. On a fallback only `fallback` and
/// `latency_ms` are set and `retrieval` names the path that answered.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct GraphFirstView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    pub anchors: Vec<String>,
    pub facts: Vec<GraphFirstFactView>,
    /// Source events of the selected claims only.
    pub events: Vec<EventView>,
    pub evidence_ids: EvidenceIdsView,
    /// Count route only: matching claims in the bounded set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<GraphFirstBudgetView>,
    pub latency_ms: f64,
    /// Why the default path answered instead: "no_anchor", "no_edges" or
    /// "temporal_constraint".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphFirstFactView {
    pub claim_id: i64,
    pub anchor: String,
    pub hop: usize,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct EvidenceIdsView {
    pub claims: Vec<i64>,
    pub events: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphFirstBudgetView {
    pub anchors: usize,
    pub max_anchors: usize,
    pub hops: usize,
    pub edges_visited: usize,
    pub max_edges: usize,
    pub candidates: usize,
    pub selected: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<String>,
    pub edges: Vec<ClaimView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AddTripleView {
    pub provenance: String,
    pub confidence: f64,
    pub object_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub triple_id: i64,
    pub status: String,
    /// See [`ClaimView::embedded`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContradictionView {
    pub contradiction_id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
    /// For the replacement claim; see [`ClaimView::embedded`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded: Option<bool>,
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
            provenance: claim.provenance.as_str().to_owned(),
            object_kind: claim.object_kind.as_str().to_owned(),
            event_id: claim.event_id,
            hint: atomicity_hint(&claim.object),
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
            embedded: None,
            context: claim.context,
            valid_from: claim.valid_from,
            valid_until: claim.valid_until,
            last_verified_at: None,
            last_confirmed_at: None,
            evidence_since_confirmation: None,
            latest_confirmation: None,
        }
    }
}

/// The optional ADR-0032 context and window of an MCP claim write.
trait ContextWindow {
    fn context_window(
        self,
        context: Option<String>,
        valid_from: Option<i64>,
        valid_until: Option<i64>,
    ) -> Self;
}

impl ContextWindow for ClaimWrite {
    fn context_window(
        self,
        context: Option<String>,
        valid_from: Option<i64>,
        valid_until: Option<i64>,
    ) -> Self {
        let write = match context {
            Some(context) => self.context(context),
            None => self,
        };
        write.valid_window(valid_from, valid_until)
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

impl From<ContradictionRecord> for ContradictionView {
    fn from(record: ContradictionRecord) -> Self {
        Self {
            contradiction_id: record.id,
            claim_id: record.claim_id,
            reason: record.reason,
            new_claim_id: record.new_claim_id,
            status: record.status.to_ascii_lowercase(),
            embedded: None,
        }
    }
}

pub struct AverTools {
    store: Store,
    /// ADR-0004 embedding client; `None` unless `AVER_EMBEDDING_URL` opts in.
    embedder: Option<Embedder>,
    /// Model label recorded on vector chunks (`AVER_EMBEDDING_MODEL`).
    embedding_model: String,
    /// ADR-0004 opt-in recency `(w, tau_secs)`; `w == 0` unless
    /// `AVER_RECALL_RECENCY_WEIGHT` opts in.
    recency: (f64, f64),
    /// ADR-0032 opt-in evidence half-life; `0` unless
    /// `AVER_RECALL_EVIDENCE_HALF_LIFE` opts in.
    evidence_half_life: f64,
    /// `AVER_EMBED_ON_WRITE=1`: claim writes also store an embedded chunk.
    embed_on_write: bool,
}

impl AverTools {
    pub fn open(memory_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let (embedder, embedding_model) = embedding_from_env();
        Ok(Self {
            store: Store::open(memory_dir)?,
            embedder,
            embedding_model,
            recency: recency_from_env(),
            evidence_half_life: evidence_half_life_from_env(),
            embed_on_write: std::env::var("AVER_EMBED_ON_WRITE").is_ok_and(|v| v.trim() == "1"),
        })
    }

    /// Replaces the `AVER_EMBED_ON_WRITE` setting read from the environment.
    pub fn with_embed_on_write(mut self, embed_on_write: bool) -> Self {
        self.embed_on_write = embed_on_write;
        self
    }

    /// Opt-in (`AVER_EMBED_ON_WRITE=1` with an embedding provider): store one
    /// vector chunk holding the canonical text of a claim that was just
    /// written, so it is eligible for hybrid recall without a separate
    /// `add_vector_chunk` call. Runs only after the claim write has passed
    /// validation and the privacy filter and is committed; nothing here can
    /// fail or undo that write. Costs one embedding request, bounded by
    /// `AVER_EMBEDDING_TIMEOUT_SECS`, made while the caller holds the tools.
    ///
    /// `None` when not opted in (the result is then unchanged); otherwise
    /// whether the chunk carries a vector.
    fn embed_written_claim(&self, claim_id: i64) -> Option<bool> {
        if !self.embed_on_write || self.embedder.is_none() {
            return None;
        }
        let written = self
            .store
            .get_claim(claim_id)
            .map_err(anyhow::Error::from)
            .and_then(|claim| self.write_chunk(claim_id, &claim.text()));
        match written {
            Ok((_, embedded)) => Some(embedded),
            Err(err) => {
                eprintln!("aver-server: claim {claim_id} saved without a vector chunk: {err}");
                Some(false)
            }
        }
    }

    /// Replaces the recency term read from the environment (tests inject
    /// weights here instead of setting `AVER_RECALL_RECENCY_WEIGHT`).
    pub fn with_recency(mut self, weight: f64, tau_secs: f64) -> anyhow::Result<Self> {
        let weights = HybridWeights::default().with_recency(weight, tau_secs)?;
        self.recency = (weights.recency_weight, weights.recency_tau_secs);
        Ok(self)
    }

    /// Replaces the evidence half-life read from the environment (tests
    /// inject it here instead of setting `AVER_RECALL_EVIDENCE_HALF_LIFE`).
    pub fn with_evidence_half_life(mut self, half_life: f64) -> anyhow::Result<Self> {
        self.evidence_half_life = HybridWeights::default()
            .with_evidence_half_life(half_life)?
            .evidence_half_life;
        Ok(self)
    }

    /// Replaces the embedding client read from the environment (tests and
    /// the BEAM parity harness inject a deterministic client here).
    pub fn with_embedding_client(mut self, embedder: Option<Embedder>) -> Self {
        self.embedder = embedder;
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
            Error::InvalidClaimField { field } | Error::InvalidEventField { field } => {
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
        let (agent_id, agent_kind) = mcp_agent(
            params.agent_id.as_deref(),
            params.agent_kind.as_deref(),
            "mcp",
            AgentKind::Llm,
        )?;
        let write = claim_options(
            ClaimWrite::from_agent(
                agent_id,
                agent_kind,
                &params.subject,
                &params.predicate,
                &params.object,
                params.source.as_deref().unwrap_or("mcp"),
            )
            .scope(params.scope.as_deref().unwrap_or("global"))
            .context_window(
                params.context.clone(),
                params.valid_from,
                params.valid_until,
            ),
            params.provenance.as_deref(),
            params.confidence,
            params.object_kind.as_deref(),
            params.event_id,
            params.strict_atomic.unwrap_or(false),
        )?;
        let id = self.store.add_claim(write)?;
        let embedded = self.embed_written_claim(id);
        Ok(ClaimView {
            embedded,
            ..self.store.get_claim(id)?.into()
        })
    }

    pub fn recall(&self, params: RecallParams) -> anyhow::Result<RecallView> {
        let graph_first = match params.mode.as_deref() {
            None | Some("hybrid") => false,
            Some("graph_first") => true,
            Some(other) => {
                anyhow::bail!("invalid mode {other:?}: use \"hybrid\" or \"graph_first\"")
            }
        };
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
            context: params.context.clone(),
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
        let weights = HybridWeights {
            recency_weight: self.recency.0,
            recency_tau_secs: self.recency.1,
            evidence_half_life: self.evidence_half_life,
            ..weights
        };
        let mut fallback = None;
        if graph_first {
            let started = std::time::Instant::now();
            let options = GraphFirstOptions {
                hops: validate_hops(params.hops.unwrap_or(2))?,
                top_k,
                weights,
            };
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            match self
                .store
                .recall_graph_first(&params.query, &filters, &options, now)?
            {
                Ok(result) => return self.graph_first_view(result, started, now),
                Err(reason) => {
                    fallback = Some(GraphFirstView {
                        fallback: Some(reason.as_str().to_string()),
                        latency_ms: elapsed_ms(started),
                        ..GraphFirstView::default()
                    })
                }
            }
        }
        let (mut claims, chunks, retrieval, alpha) =
            match self.recall_hybrid(&params.query, &filters, top_k, weights)? {
                Some((claims, chunks)) => (claims, chunks, "hybrid", Some(weights.alpha)),
                None => (
                    self.store
                        .recall_text(RecallQuery::new(&params.query).filters(filters))?,
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
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let triples = claims
            .into_iter()
            .map(|claim| self.confirmed_view(claim, now))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(RecallView {
            triples,
            chunks,
            subgraph: GraphView {
                nodes: subgraph.nodes,
                edges: subgraph.edges.into_iter().map(ClaimView::from).collect(),
            },
            confidence_floor,
            retrieval: retrieval.to_string(),
            alpha,
            graph_first: fallback,
        })
    }

    /// ADR-0033 answer: the selected claims as triples, the bounded
    /// expansion as the subgraph, and the evidence and budget beside them.
    fn graph_first_view(
        &self,
        result: GraphFirstRecall,
        started: std::time::Instant,
        now: i64,
    ) -> anyhow::Result<RecallView> {
        let confidence_floor = result
            .facts
            .iter()
            .map(|fact| &fact.claim)
            .chain(result.subgraph.edges.iter())
            .map(|claim| claim.confidence)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        let facts = result
            .facts
            .iter()
            .map(|fact| GraphFirstFactView {
                claim_id: fact.claim.id,
                anchor: fact.anchor.clone(),
                hop: fact.hop,
                ts: fact.ts,
            })
            .collect();
        let evidence_ids = EvidenceIdsView {
            claims: result.facts.iter().map(|fact| fact.claim.id).collect(),
            events: result.events.iter().map(|event| event.id).collect(),
        };
        let triples = result
            .facts
            .into_iter()
            .map(|fact| self.confirmed_view(fact.claim, now))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let budget = result.budget;
        Ok(RecallView {
            triples,
            chunks: Vec::new(),
            subgraph: GraphView {
                nodes: result.subgraph.nodes,
                edges: result
                    .subgraph
                    .edges
                    .into_iter()
                    .map(ClaimView::from)
                    .collect(),
            },
            confidence_floor,
            retrieval: "graph_first".to_string(),
            alpha: None,
            graph_first: Some(GraphFirstView {
                route: Some(result.route.as_str().to_string()),
                anchors: result.anchors,
                facts,
                events: result.events.into_iter().map(EventView::from).collect(),
                evidence_ids,
                count: result.count,
                budget: Some(GraphFirstBudgetView {
                    anchors: budget.anchors,
                    max_anchors: budget.max_anchors,
                    hops: budget.hops,
                    edges_visited: budget.edges_visited,
                    max_edges: budget.max_edges,
                    candidates: budget.candidates,
                    selected: budget.selected,
                    truncated: budget.truncated,
                }),
                latency_ms: elapsed_ms(started),
                fallback: None,
            }),
        })
    }

    /// ADR-0004 hybrid half of [`AverTools::recall`]: claims ranked by core's
    /// `recall_hybrid_claims_with_filters` (scope, agent, predicate,
    /// min_confidence and status filters applied in core), plus the
    /// vector-chunk texts of those claims ranked by similarity to the query.
    ///
    /// `Ok(None)` means "use the keyword path": no embedding client, no
    /// embedded chunks, or the embedding call failed.
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
        if self.store.vector_chunk_embedding_status()?.0 == 0 {
            return Ok(None);
        }
        let query_embedding = match embedder.embed(query) {
            Ok(embedding) => embedding,
            Err(err) => {
                eprintln!("aver-server: query embedding failed, using keyword recall: {err}");
                return Ok(None);
            }
        };
        let claims = match self.store.recall_hybrid_claims_with_filters(
            query,
            &query_embedding,
            top_k,
            weights,
            filters,
            time::OffsetDateTime::now_utc().unix_timestamp(),
        ) {
            Ok(claims) => claims,
            Err(Error::InvalidEmbeddingVector) => {
                eprintln!(
                    "aver-server: query embedding failed, using keyword recall: invalid embedding vector"
                );
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };
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
        let (agent_id, agent_kind) = mcp_agent(
            params.agent_id.as_deref(),
            params.agent_kind.as_deref(),
            "local",
            AgentKind::Llm,
        )?;
        let write = claim_options(
            ClaimWrite::from_agent(
                agent_id,
                agent_kind,
                &params.subject,
                &params.predicate,
                &params.object,
                &params.source,
            )
            .scope(params.scope.as_deref().unwrap_or("global"))
            .context_window(
                params.context.clone(),
                params.valid_from,
                params.valid_until,
            ),
            params.provenance.as_deref(),
            params.confidence,
            params.object_kind.as_deref(),
            params.event_id,
            params.strict_atomic.unwrap_or(false),
        )?;
        let id = self.store.add_claim(write)?;
        let claim = self.store.get_claim(id)?;
        Ok(AddTripleView {
            triple_id: id,
            status: "appended".to_string(),
            embedded: self.embed_written_claim(id),
            provenance: claim.provenance.as_str().to_owned(),
            confidence: claim.confidence,
            object_kind: claim.object_kind.as_str().to_owned(),
            event_id: claim.event_id,
            hint: atomicity_hint(&claim.object),
        })
    }

    pub fn contradict(&self, params: ContradictParams) -> anyhow::Result<ContradictionView> {
        let new_claim = match params.new_claim.as_ref() {
            None => None,
            Some(claim) => {
                let (agent_id, agent_kind) = mcp_agent(
                    claim.agent_id.as_deref(),
                    claim.agent_kind.as_deref(),
                    "local",
                    AgentKind::Llm,
                )?;
                // The replacement inherits the contradicted claim's scope
                // unless the caller says otherwise.
                let scope = match claim.scope.as_deref() {
                    Some(scope) => scope.to_string(),
                    None => self.store.get_claim(params.triple_id)?.scope,
                };
                Some(
                    ClaimWrite::from_agent(
                        agent_id,
                        agent_kind,
                        &claim.subject,
                        &claim.predicate,
                        &claim.object,
                        &claim.source,
                    )
                    .scope(scope),
                )
            }
        };
        let record = self
            .store
            .contradict(params.triple_id, &params.reason, new_claim)?;
        let embedded = record
            .new_claim_id
            .and_then(|id| self.embed_written_claim(id));
        Ok(ContradictionView {
            embedded,
            ..record.into()
        })
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

    /// Read one source event only inside the exact requested memory scope.
    /// Missing and out-of-scope IDs deliberately share an actionable error.
    pub fn get_event(&self, params: GetEventParams) -> anyhow::Result<EventView> {
        let requested = params.scope.as_deref().unwrap_or("global");
        if requested.trim().is_empty() {
            anyhow::bail!("invalid scope: use the exact event scope returned by record_event");
        }
        let scope =
            crate::scope_resolution::resolve_scope(Some(requested), None, None, None)?.scope;
        match self.store.get_event(params.event_id) {
            Ok(event) if event.scope == scope => Ok(event.into()),
            Ok(_) | Err(Error::MissingEvent { .. }) => anyhow::bail!(
                "source event unavailable in requested scope; use an event_id returned by record_event and its exact scope"
            ),
            Err(err) => Err(err.into()),
        }
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
        let id = self.store.record_event(
            EventWrite::new(&params.session_id, &params.kind, &params.payload, source)
                .agent(agent_id, agent_kind)
                .scope(scope),
        )?;
        Ok(self.store.get_event(id)?.into())
    }

    pub fn should_extract_memories(
        &self,
        params: ShouldExtractMemoriesParams,
    ) -> anyhow::Result<ShouldExtractMemoriesView> {
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
        let id = self.store.propose_candidate_claim(
            CandidateWrite::new(
                params.event_id,
                &params.subject,
                &params.predicate,
                &params.object,
            )
            .scope(scope),
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
        let embedded = self.embed_written_claim(claim_id);
        Ok(ClaimView {
            embedded,
            ..self.store.get_claim(claim_id)?.into()
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

    /// ADR-0032: a recall triple with its confirmation state, filled only for
    /// a claim with a context or an explicit confirmation.
    fn confirmed_view(&self, claim: Claim, now: i64) -> anyhow::Result<ClaimView> {
        let confirmation = self.store.claim_confirmation(&claim, now)?;
        let shown = claim.context.is_some() || confirmation.latest.is_some();
        let last_verified_at = claim.last_verified_at;
        let mut view = ClaimView::from(claim);
        if shown {
            view.last_verified_at = last_verified_at;
            view.last_confirmed_at = Some(confirmation.last_confirmed_at);
            view.evidence_since_confirmation = confirmation.evidence_since_confirmation;
            view.latest_confirmation = confirmation.latest.map(|latest| ConfirmationView {
                at: latest.verified_at,
                agent_id: latest.agent_id,
                agent_kind: latest.agent_kind.as_str().to_string(),
                evidence: latest.evidence,
            });
        }
        Ok(view)
    }

    /// ADR-0032 `reverify_claim`: record that a claim was re-confirmed by the
    /// cited evidence, naming the confirming writer.
    pub fn reverify_claim(&self, params: ReverifyClaimParams) -> anyhow::Result<ReverifyClaimView> {
        let (agent_id, agent_kind) = mcp_agent(
            params.agent_id.as_deref(),
            params.agent_kind.as_deref(),
            "mcp",
            AgentKind::Llm,
        )?;
        let mut write = ReverifyWrite::new(params.claim_id, params.evidence.clone())
            .agent(agent_id, agent_kind)
            .scope(params.scope.as_deref().unwrap_or("global"));
        if let Some(valid_until) = params.valid_until {
            write = write.valid_until(valid_until);
        }
        let verified_at = self.store.reverify_claim(write)?;
        Ok(ReverifyClaimView {
            claim_id: params.claim_id,
            verified_at,
            agent_id: agent_id.to_string(),
            agent_kind: agent_kind.as_str().to_string(),
            evidence: params.evidence,
            valid_until: self.store.get_claim(params.claim_id)?.valid_until,
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
        let (chunk_id, _) = self.write_chunk(params.claim_id, &params.text)?;
        Ok(VectorChunkView {
            id: chunk_id,
            claim_id: params.claim_id,
            text: params.text,
        })
    }

    /// Stores a vector chunk and reports whether it carries an embedding.
    fn write_chunk(&self, claim_id: i64, text: &str) -> anyhow::Result<(i64, bool)> {
        // Embed only text that core's validation and privacy filter would
        // accept, so rejected text never leaves the process. Without a client,
        // or when embedding fails, the chunk is stored text-only.
        let embedding = self
            .embedder
            .as_deref()
            .filter(|_| !text.trim().is_empty() && aver_core::privacy_filter(text).is_ok())
            .and_then(|embedder| match embedder.embed(text) {
                Ok(embedding) => Some(embedding),
                Err(err) => {
                    eprintln!("aver-server: chunk embedding failed, storing text only: {err}");
                    None
                }
            });
        Ok(match embedding {
            Some(embedding) => match self.store.add_vector_chunk_with_embedding(
                claim_id,
                text,
                &self.embedding_model,
                &embedding,
            ) {
                Ok(chunk_id) => (chunk_id, true),
                Err(Error::InvalidEmbeddingVector) => (
                    self.store
                        .add_vector_chunk(claim_id, text, &self.embedding_model)?,
                    false,
                ),
                Err(err) => return Err(err.into()),
            },
            None => (
                self.store
                    .add_vector_chunk(claim_id, text, &self.embedding_model)?,
                false,
            ),
        })
    }
}

/// Resolve `ScopeWalk` from optional MCP request strings.
///
/// Agent attribution defaults to LLM for durable MCP claims. Explicit human
/// assertions and deterministic/source-backed extraction remain available.
fn mcp_agent<'a>(
    agent_id: Option<&'a str>,
    agent_kind: Option<&str>,
    default_agent_id: &'a str,
    default_agent_kind: AgentKind,
) -> anyhow::Result<(&'a str, AgentKind)> {
    let agent_kind = match agent_kind {
        Some(kind) => kind.parse::<AgentKind>()?,
        None => default_agent_kind,
    };
    Ok((agent_id.unwrap_or(default_agent_id), agent_kind))
}

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

/// Server-side traversal cap. Core rejects `hops == 0`
/// (`Error::InvalidGraphHops`) but has no upper bound.
fn validate_hops(hops: usize) -> anyhow::Result<usize> {
    if hops <= 8 {
        Ok(hops)
    } else {
        anyhow::bail!("invalid hops: must be at most 8")
    }
}

fn elapsed_ms(started: std::time::Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
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

fn atomicity_hint(object: &str) -> Option<String> {
    aver_core::ObjectKind::is_prose(object).then(|| "For atomic memory, split prose into durable triples using stable IDs; keep rationale in record_event and cite its event_id. Use strict_atomic=true to reject prose.".to_owned())
}

fn claim_options(
    mut write: ClaimWrite,
    provenance: Option<&str>,
    confidence: Option<f64>,
    object_kind: Option<&str>,
    event_id: Option<i64>,
    strict_atomic: bool,
) -> anyhow::Result<ClaimWrite> {
    if let Some(value) = provenance {
        write = write.provenance(value.parse()?);
    }
    if let Some(value) = confidence {
        write = write.confidence(value);
    }
    if let Some(value) = object_kind {
        write = write.object_kind(value.parse()?);
    }
    if let Some(value) = event_id {
        write = write.event_id(value);
    }
    Ok(write.strict_atomic(strict_atomic))
}
