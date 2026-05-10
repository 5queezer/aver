use std::collections::BTreeMap;
use std::str::FromStr;

use crate::Error;

/// A claim row as exposed to consumers (ADR-0003).
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionRecord {
    pub id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewClaim<'a> {
    pub subject: &'a str,
    pub predicate: &'a str,
    pub object: &'a str,
    pub source: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Community {
    pub id: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMode {
    Local,
    Shared,
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
    SessionEnd,
    Correction,
    CommitCompleted,
    IdleCompaction,
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
