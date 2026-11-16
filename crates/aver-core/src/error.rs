//! Crate error type.

use crate::privacy::PrivacyRejection;
use crate::vector;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("embedding: {0}")]
    Embedding(#[from] vector::EmbeddingError),
    #[error("privacy filter rejected content: {0}")]
    Privacy(#[from] PrivacyRejection),
    #[error("invalid {kind} value in database: {value:?}")]
    EnumParse { kind: &'static str, value: String },
    #[error("invalid agent_id for partitioned log path: {value:?}")]
    InvalidAgentId { value: String },
    #[error("invalid scope: {value:?}; must be non-blank and match [A-Za-z0-9_/-]")]
    InvalidScope { value: String },
    #[error("claim {claim_id} is already INVALIDATED")]
    AlreadyRetired { claim_id: i64 },
    #[error("invalid vector chunk text: must not be empty")]
    InvalidVectorChunkText,
    #[error("invalid embedding model: must not be empty")]
    InvalidEmbeddingModel,
    #[error("invalid embedding vector: must not be empty")]
    InvalidEmbeddingVector,
    #[error("invalid claim {field}: must not be empty")]
    InvalidClaimField { field: &'static str },
    #[error("invalid event {field}: must not be empty")]
    InvalidEventField { field: &'static str },
    #[error("invalid observation {field}: must not be empty")]
    InvalidObservationField { field: &'static str },
    #[error("invalid recall query: must not be empty")]
    InvalidRecallQuery,
    #[error("invalid top_k: must be greater than zero")]
    InvalidTopK,
    #[error("invalid graph entity: must not be empty")]
    InvalidGraphEntity,
    #[error("invalid graph hops: must be greater than zero")]
    InvalidGraphHops,
    #[error("invalid predicate filter: must not be empty")]
    InvalidPredicateFilter,
    #[error("invalid event threshold: must be greater than zero")]
    InvalidEventThreshold,
    #[error("invalid rejection reason: must not be empty")]
    InvalidRejectionReason,
    #[error("invalid contradiction reason: must not be empty")]
    InvalidContradictionReason,
    #[error(
        "object is prose or a scalar literal; use object_kind=literal and split durable facts into atomic triples, keeping rationale in record_event"
    )]
    InvalidEntityObject,
    #[error(
        "prose object rejected in strict_atomic mode; split it into atomic triples and store rationale with record_event, then cite event_id"
    )]
    NonAtomicObject,
    #[error("claim event_id must reference an existing event in the claim scope")]
    InvalidClaimEvent,
    #[error("invalid confidence value: {value}")]
    InvalidConfidence { value: f64 },
    #[error("invalid decay tau: {value}")]
    InvalidDecayTau { value: f64 },
    #[error("candidate claim must cite an existing event: {event_id}")]
    MissingEventProvenance { event_id: i64 },
    #[error("missing event: event {event_id} does not exist")]
    MissingEvent { event_id: i64 },
    #[error("missing candidate claim: candidate {candidate_id} does not exist")]
    MissingCandidate { candidate_id: i64 },
    #[error("missing observation: observation {observation_id} does not exist")]
    MissingObservation { observation_id: String },
    #[error("observation id collision: {observation_id} already exists with different content")]
    ObservationIdCollision { observation_id: String },
    #[error("missing entity type: {name} (ontology bootstrap incomplete)")]
    MissingEntityType { name: &'static str },
    #[error("invalid candidate claim status for candidate {candidate_id}: {status}")]
    InvalidCandidateStatus { candidate_id: i64, status: String },
    #[error("invalid candidate status filter: {status}")]
    InvalidCandidateStatusFilter { status: String },
    #[error("missing entity: {entity}")]
    MissingEntity { entity: String },
    #[error("unknown predicate: {name} (not in predicate_types or predicate_alias)")]
    UnknownPredicate { name: String },
    #[error("ontology extension {path}: {detail}")]
    OntologyFile { path: String, detail: String },
    #[error("invalid claim validity: {detail}")]
    InvalidClaimValidity { detail: String },
    #[error("reverify_claim rejected: {detail}")]
    InvalidReverify { detail: String },
    #[error("missing claim: claim {claim_id} does not exist")]
    MissingClaim { claim_id: i64 },
    #[error("missing vector chunk: vector chunk {chunk_id} does not exist")]
    MissingVectorChunk { chunk_id: i64 },
    #[error(
        "schema too new: db user_version is {found}, this binary supports {supported}; refusing to open"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("replay: duplicate id with conflicting content: {detail}")]
    ReplayDuplicateId { detail: String },
    #[error("invalid log record: {detail}")]
    InvalidRecord { detail: String },
    #[error("replay: unknown record kind: {kind}")]
    ReplayUnknownKind { kind: String },
    #[error("replay: malformed log record at {path}:{line}: {detail}")]
    ReplayMalformed {
        path: String,
        line: usize,
        detail: String,
    },
    #[error("advisory lock held: {path}")]
    LockHeld { path: String },
}
