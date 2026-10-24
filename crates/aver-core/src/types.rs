use std::collections::BTreeMap;
use std::str::FromStr;

use crate::Error;

/// A claim row as exposed to consumers (ADR-0003, ADR-0021).
#[derive(Debug, Clone)]
pub struct Claim {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub status: ClaimStatus,
    pub source_refs: Vec<String>,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub write_ts: i64,
    pub last_verified_at: Option<i64>,
    /// ADR-0021 hierarchical memory scope (e.g. "global", "proj/aver",
    /// "proj/aver/branch/feat_x"). Defaults to "global" when unspecified.
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HyperedgeParticipantInput {
    pub role: String,
    pub entity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperedgeInput {
    pub predicate: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub source_refs: Vec<String>,
    pub participants: Vec<HyperedgeParticipantInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperedgeParticipant {
    pub id: i64,
    pub hyperedge_id: i64,
    pub role: String,
    pub entity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hyperedge {
    pub id: i64,
    pub predicate: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub source_refs: Vec<String>,
    pub status: ClaimStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub participants: Vec<HyperedgeParticipant>,
}

impl Claim {
    pub fn text(&self) -> String {
        format!("{} {} {}", self.subject, self.predicate, self.object)
    }

    pub fn verification_weighted_confidence(&self) -> f64 {
        if self.last_verified_at.is_some() {
            self.confidence
        } else {
            self.confidence * 0.5
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpisodicEvent {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub ts: i64,
    /// ADR-0021 memory scope. Defaults to "global".
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateClaim {
    pub id: i64,
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub status: String,
    pub promoted_claim_id: Option<i64>,
    pub rejection_reason: Option<String>,
    /// ADR-0021 memory scope. Inherits to the promoted claim. Defaults to "global".
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateClaimDraft {
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationRelevance {
    Low,
    Medium,
    High,
    Critical,
}

impl ObservationRelevance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

impl FromStr for ObservationRelevance {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            other => Err(Error::EnumParse {
                kind: "ObservationRelevance",
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub id: String,
    pub session_id: String,
    pub content: String,
    pub relevance: ObservationRelevance,
    pub source_event_ids: Vec<i64>,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub derivation: String,
    pub ts: i64,
    /// ADR-0021 memory scope. Defaults to "global".
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationDraft {
    pub content: String,
    pub relevance: ObservationRelevance,
    pub source_event_ids: Vec<i64>,
    pub derivation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservationRecall {
    pub observation: Observation,
    pub events: Vec<EpisodicEvent>,
    pub audit_status: Option<String>,
    pub prune_marker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObservationCoverage {
    pub event_ids: Vec<i64>,
    pub covered_event_ids: Vec<i64>,
    pub uncovered_event_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct GraphExpansion {
    pub nodes: Vec<String>,
    pub edges: Vec<Claim>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPathMode {
    Directed,
    Bidirectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipKind {
    Claim,
    Hyperedge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPathQuery {
    pub source: String,
    pub target: String,
    pub min_confidence: f64,
    pub allowed_provenance: Option<Vec<Provenance>>,
    pub max_hops: usize,
    pub predicates: Option<Vec<String>>,
    pub mode: GraphPathMode,
    /// ADR-0021 scope the traversal is evaluated from. Claims are admitted
    /// per `walk`; hyperedges carry no scope column and are treated as
    /// `"global"` rows (see `Store::graph_path`).
    pub scope: String,
    /// ADR-0021 scope walk. `ScopeWalk::Any` ignores `scope` and reproduces
    /// the unscoped traversal.
    pub walk: ScopeWalk,
}

impl Default for GraphPathQuery {
    /// Unscoped directed query with no confidence floor and 8 hops; `source`
    /// and `target` are empty and must be set by the caller.
    fn default() -> Self {
        Self {
            source: String::new(),
            target: String::new(),
            min_confidence: 0.0,
            allowed_provenance: None,
            max_hops: 8,
            predicates: None,
            mode: GraphPathMode::Directed,
            scope: "global".to_string(),
            walk: ScopeWalk::Any,
        }
    }
}

impl GraphPathQuery {
    /// Query from `source` to `target` with the `Default` settings.
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPathStep {
    pub source: String,
    pub target: String,
    pub predicate: String,
    pub confidence: f64,
    pub provenance: Provenance,
    pub relationship_kind: RelationshipKind,
    pub source_refs: Vec<String>,
    /// Id of the claim behind this step; `None` for hyperedge steps.
    pub claim_id: Option<i64>,
    /// ADR-0021 scope of the row behind this step. Hyperedges have no scope
    /// column and report `"global"`.
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPath {
    pub source: String,
    pub target: String,
    pub steps: Vec<GraphPathStep>,
    pub confidence: f64,
    entity_path: Vec<String>,
}

impl GraphPath {
    pub(crate) fn new(
        source: String,
        target: String,
        steps: Vec<GraphPathStep>,
        confidence: f64,
        entity_path: Vec<String>,
    ) -> Self {
        Self {
            source,
            target,
            steps,
            confidence,
            entity_path,
        }
    }

    pub fn entities(&self) -> Vec<&str> {
        self.entity_path.iter().map(String::as_str).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionRecord {
    pub id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
    pub created_at: i64,
}

/// One claim write (ADR-0026 one write path). Build with [`ClaimWrite::new`]
/// (local human, USER_ASSERTED, 0.95, scope "global") or
/// [`ClaimWrite::from_agent`], then override with the builder setters.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimWrite {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub provenance: Provenance,
    pub confidence: f64,
    /// ADR-0021 memory scope; `"global"` unless set.
    pub scope: String,
}

impl ClaimWrite {
    /// A USER_ASSERTED claim by the local human (`agent_id` "local",
    /// confidence 0.95, scope "global").
    pub fn new(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self::from_agent(
            "local",
            AgentKind::Human,
            subject,
            predicate,
            object,
            source,
        )
    }

    /// A claim attributed to `agent_id`/`agent_kind`, with the provenance and
    /// policy confidence that kind implies (ADR-0003).
    pub fn from_agent(
        agent_id: impl Into<String>,
        agent_kind: AgentKind,
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        let provenance = agent_kind.default_provenance();
        Self {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            source: source.into(),
            agent_id: agent_id.into(),
            agent_kind,
            provenance,
            confidence: provenance.policy_confidence(),
            scope: "global".to_string(),
        }
    }

    /// Override the provenance; the confidence follows the new provenance's
    /// policy value unless [`ClaimWrite::confidence`] is set afterwards.
    pub fn provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self.confidence = provenance.policy_confidence();
        self
    }

    pub fn confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }
}

/// One episodic event write (ADR-0014). Defaults: `agent_id` "local",
/// `AgentKind::Human`, scope "global".
#[derive(Debug, Clone, PartialEq)]
pub struct EventWrite {
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub scope: String,
}

impl EventWrite {
    pub fn new(
        session_id: impl Into<String>,
        kind: impl Into<String>,
        payload: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            kind: kind.into(),
            payload: payload.into(),
            source: source.into(),
            agent_id: "local".to_string(),
            agent_kind: AgentKind::Human,
            scope: "global".to_string(),
        }
    }

    pub fn agent(mut self, agent_id: impl Into<String>, agent_kind: AgentKind) -> Self {
        self.agent_id = agent_id.into();
        self.agent_kind = agent_kind;
        self
    }

    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }
}

/// One observation write (ADR-0016). The agent attribution comes from the
/// first source event; scope defaults to "global".
#[derive(Debug, Clone, PartialEq)]
pub struct ObservationWrite {
    pub session_id: String,
    pub content: String,
    pub relevance: ObservationRelevance,
    pub source_event_ids: Vec<i64>,
    pub derivation: String,
    pub scope: String,
}

impl ObservationWrite {
    pub fn new(
        session_id: impl Into<String>,
        content: impl Into<String>,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            content: content.into(),
            relevance,
            source_event_ids: source_event_ids.to_vec(),
            derivation: derivation.into(),
            scope: "global".to_string(),
        }
    }

    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }
}

/// One staged candidate claim (ADR-0005/0014), citing the event it was
/// extracted from. Scope defaults to "global" and rides onto the promoted
/// claim.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateWrite {
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub scope: String,
}

impl CandidateWrite {
    pub fn new(
        event_id: i64,
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
    ) -> Self {
        Self {
            event_id,
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            scope: "global".to_string(),
        }
    }

    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractorFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Option<Provenance>,
}

impl ExtractorFact {
    pub fn new(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            provenance: None,
        }
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = Some(provenance);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Community {
    pub id: String,
    pub members: Vec<String>,
    /// Deterministic cohesion score for the community, currently the average
    /// internal relationship weight after confidence/provenance weighting.
    pub score: f64,
    /// Member nodes that also touch relationships outside the community.
    pub bridge_nodes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMode {
    Local,
    Shared,
}

/// ADR-0023 §"recall and expand": how a predicate filter expands through the
/// `predicate_closure` (ADR-0010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PredicateWalk {
    /// Match only rows whose `predicate` equals the input.
    #[default]
    Exact,
    /// Match input plus every predicate that has the input as an ancestor.
    Descendants,
}

impl PredicateWalk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Descendants => "descendants",
        }
    }
}

impl FromStr for PredicateWalk {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "exact" => Ok(Self::Exact),
            "descendants" => Ok(Self::Descendants),
            other => Err(Error::EnumParse {
                kind: "PredicateWalk",
                value: other.to_string(),
            }),
        }
    }
}

/// ADR-0023 — typed filter set for `recall_text_with_filters` and
/// `expand_with_filters`. Constructed via `Default::default()` and then
/// updated by the caller; this preserves the pre-Layer-3 behavior of
/// `recall_text` (scope="global", walk=any, status=ACTIVE) when defaults
/// are accepted.
#[derive(Debug, Clone)]
pub struct RecallFilters {
    pub scope: String,
    pub scope_walk: ScopeWalk,
    pub agent_id: Option<String>,
    pub agent_kind: Option<AgentKind>,
    pub predicate: Option<String>,
    pub predicate_walk: PredicateWalk,
    pub min_confidence: Option<f64>,
    /// `Some(status)` matches that status exactly. `None` means "any status"
    /// (drops the implicit `WHERE status='ACTIVE'` clause).
    pub status: Option<ClaimStatus>,
}

impl Default for RecallFilters {
    fn default() -> Self {
        Self {
            scope: "global".to_string(),
            scope_walk: ScopeWalk::Any,
            agent_id: None,
            agent_kind: None,
            predicate: None,
            predicate_walk: PredicateWalk::Exact,
            min_confidence: None,
            status: Some(ClaimStatus::Active),
        }
    }
}

/// One keyword-recall request: the query text, the ADR-0021/0023 filters
/// and an optional result cap. `RecallQuery::new(q)` reproduces the
/// unfiltered, uncapped search.
#[derive(Debug, Clone)]
pub struct RecallQuery {
    pub query: String,
    pub filters: RecallFilters,
    /// `None` returns every match in rank order.
    pub top_k: Option<usize>,
}

impl RecallQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            filters: RecallFilters::default(),
            top_k: None,
        }
    }

    /// Restrict to `scope` under `walk` (keeps the other filters).
    pub fn scope(mut self, scope: impl Into<String>, walk: ScopeWalk) -> Self {
        self.filters.scope = scope.into();
        self.filters.scope_walk = walk;
        self
    }

    pub fn filters(mut self, filters: RecallFilters) -> Self {
        self.filters = filters;
        self
    }

    pub fn top_k(mut self, top_k: usize) -> Self {
        self.top_k = Some(top_k);
        self
    }
}

/// ADR-0021 §"Read-path semantics": how a scope filter walks the path tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeWalk {
    /// Only rows whose scope equals the input.
    Exact,
    /// Input scope plus every path prefix up to "global". Default per ADR-0021.
    #[default]
    Ancestors,
    /// Input scope plus every path beginning with `input/`.
    Descendants,
    /// No scope filter applied.
    Any,
}

impl ScopeWalk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Ancestors => "ancestors",
            Self::Descendants => "descendants",
            Self::Any => "any",
        }
    }
}

impl FromStr for ScopeWalk {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "exact" => Ok(Self::Exact),
            "ancestors" => Ok(Self::Ancestors),
            "descendants" => Ok(Self::Descendants),
            "any" => Ok(Self::Any),
            other => Err(Error::EnumParse {
                kind: "ScopeWalk",
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsolidationReport {
    pub merged: usize,
    pub superseded: usize,
    pub decayed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionTriggerReason {
    ExplicitRemember,
    EventCountThreshold,
    ObservationTokenThreshold,
    UncoveredCoverageGap,
    SessionEnd,
    Correction,
    CommitCompleted,
    IdleCompaction,
    /// ADR-0024: a decision model chose `extract` for undigested events.
    DecisionModel,
}

impl ExtractionTriggerReason {
    /// Stable snake_case name. For the five event-driven reasons this is
    /// also the episodic event `kind` that triggers them.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExplicitRemember => "explicit_remember",
            Self::EventCountThreshold => "event_count_threshold",
            Self::ObservationTokenThreshold => "observation_token_threshold",
            Self::UncoveredCoverageGap => "uncovered_coverage_gap",
            Self::SessionEnd => "session_end",
            Self::Correction => "correction",
            Self::CommitCompleted => "commit_completed",
            Self::IdleCompaction => "idle_compaction",
            Self::DecisionModel => "decision_model",
        }
    }
}

#[cfg(test)]
mod extraction_trigger_reason_tests {
    use super::ExtractionTriggerReason;

    #[test]
    fn as_str_is_unique_and_matches_the_historical_literals() {
        let expected = [
            (
                ExtractionTriggerReason::ExplicitRemember,
                "explicit_remember",
            ),
            (
                ExtractionTriggerReason::EventCountThreshold,
                "event_count_threshold",
            ),
            (
                ExtractionTriggerReason::ObservationTokenThreshold,
                "observation_token_threshold",
            ),
            (
                ExtractionTriggerReason::UncoveredCoverageGap,
                "uncovered_coverage_gap",
            ),
            (ExtractionTriggerReason::SessionEnd, "session_end"),
            (ExtractionTriggerReason::Correction, "correction"),
            (ExtractionTriggerReason::CommitCompleted, "commit_completed"),
            (ExtractionTriggerReason::IdleCompaction, "idle_compaction"),
            (ExtractionTriggerReason::DecisionModel, "decision_model"),
        ];
        let mut seen = std::collections::HashSet::new();
        for (reason, literal) in expected {
            assert_eq!(reason.as_str(), literal);
            assert!(seen.insert(reason.as_str()), "duplicate name {literal}");
        }
        assert_eq!(seen.len(), 9);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionDecision {
    pub should_extract: bool,
    pub reasons: Vec<ExtractionTriggerReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphDriftSnapshot {
    pub claim_count_by_provenance: BTreeMap<String, u64>,
    pub mean_confidence_by_provenance: BTreeMap<String, f64>,
    pub contradicts_edge_count: u64,
    pub ambiguous_ratio: f64,
    pub entity_count_by_type_id: BTreeMap<String, u64>,
    pub consolidation_merged: usize,
    pub consolidation_superseded: usize,
    pub privacy_rejection_counts: BTreeMap<String, u64>,
}

/// A text chunk attached to a claim for vector indexing.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorChunk {
    pub id: i64,
    pub claim_id: i64,
    pub text: String,
    pub embedding_model: String,
    pub embedding: Option<Vec<f32>>,
}

/// Writer class for shared-mode agent provenance (ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Human,
    Llm,
    DeterministicParser,
    ExternalTool,
}

impl AgentKind {
    /// Provenance a claim gets when its writer does not say otherwise
    /// (ADR-0003 policy table).
    pub fn default_provenance(self) -> Provenance {
        match self {
            Self::Human => Provenance::UserAsserted,
            Self::DeterministicParser | Self::ExternalTool => Provenance::Extracted,
            Self::Llm => Provenance::Inferred,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "HUMAN",
            Self::Llm => "LLM",
            Self::DeterministicParser => "DETERMINISTIC_PARSER",
            Self::ExternalTool => "EXTERNAL_TOOL",
        }
    }
}

impl FromStr for AgentKind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "HUMAN" => Ok(Self::Human),
            "LLM" => Ok(Self::Llm),
            "DETERMINISTIC_PARSER" => Ok(Self::DeterministicParser),
            "EXTERNAL_TOOL" => Ok(Self::ExternalTool),
            other => Err(Error::EnumParse {
                kind: "AgentKind",
                value: other.to_string(),
            }),
        }
    }
}

/// How a claim was acquired (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    UserAsserted,
    Extracted,
    Inferred,
    Ambiguous,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserAsserted => "USER_ASSERTED",
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }

    pub fn policy_confidence(self) -> f64 {
        match self {
            Self::UserAsserted => 0.95,
            Self::Extracted => 0.90,
            Self::Inferred => 0.45,
            Self::Ambiguous => 0.20,
        }
    }
}

impl FromStr for Provenance {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "USER_ASSERTED" => Ok(Self::UserAsserted),
            "EXTRACTED" => Ok(Self::Extracted),
            "INFERRED" => Ok(Self::Inferred),
            "AMBIGUOUS" => Ok(Self::Ambiguous),
            other => Err(Error::EnumParse {
                kind: "Provenance",
                value: other.to_string(),
            }),
        }
    }
}

/// Lifecycle status of a claim (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    Active,
    Superseded,
    Invalidated,
}

impl ClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Superseded => "SUPERSEDED",
            Self::Invalidated => "INVALIDATED",
        }
    }
}

impl FromStr for ClaimStatus {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "ACTIVE" => Ok(Self::Active),
            "SUPERSEDED" => Ok(Self::Superseded),
            "INVALIDATED" => Ok(Self::Invalidated),
            other => Err(Error::EnumParse {
                kind: "ClaimStatus",
                value: other.to_string(),
            }),
        }
    }
}
