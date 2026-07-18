//! Aver core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

mod candidates;
mod claims;
mod episodic;
mod error;
pub mod extractor;
mod graph;
mod hyperedges;
mod log;
mod maintenance;
mod ontology;
mod privacy;
mod recall;
mod replay;
pub mod retrieval;
mod seed;
mod store;
mod types;
mod validation;
pub mod vector;
mod vectors;

pub use candidates::{ClaimExtractor, MockClaimExtractor};
pub use episodic::{MockObserver, Observer};
pub use error::Error;
pub use graph::GraphStorageAdapter;
pub use log::{AverLock, LOG_ROTATE_MAX_BYTES, LOG_ROTATE_MAX_LINES};
pub use maintenance::{VacuumReport, vacuum};
pub use privacy::{PrivacyRejection, privacy_filter, privacy_filter_path};
pub use replay::{ReplayMode, ReplayQuarantine, ReplayReport, replay, replay_with_mode};
pub use store::{SqliteVecStatus, Store, VECTOR_INDEX_DIM};
pub use types::{
    AgentKind, CandidateClaim, CandidateClaimDraft, Claim, ClaimStatus, Community,
    ConsolidationReport, ContradictionRecord, EpisodicEvent, ExtractionDecision,
    ExtractionTriggerReason, ExtractorFact, GraphDriftSnapshot, GraphExpansion, GraphPath,
    GraphPathMode, GraphPathQuery, GraphPathStep, Hyperedge, HyperedgeInput, HyperedgeParticipant,
    HyperedgeParticipantInput, NewClaim, Observation, ObservationCoverage, ObservationDraft,
    ObservationRecall, ObservationRelevance, PredicateWalk, Provenance, RecallFilters,
    RelationshipKind, ScopeWalk, StorageMode, VectorChunk,
};
