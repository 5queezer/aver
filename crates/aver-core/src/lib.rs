//! Aver core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

mod apply;
mod candidates;
mod claims;
mod episodic;
mod error;
mod graph;
mod ids;
mod log;
mod maintenance;
mod objects;
mod ontology;
mod privacy;
mod recall;
mod replay;
pub mod retrieval;
mod seed;
mod store;
mod types;
mod validation;
mod validity;
pub mod vector;
mod vectors;

pub use error::Error;
pub use log::{AverLock, LOG_ROTATE_MAX_BYTES, LOG_ROTATE_MAX_LINES};
pub use maintenance::{VacuumReport, vacuum};
pub use objects::{LiteralMigrationReport, ObjectKind};
pub use ontology::PredicateSuggestion;
pub use privacy::{PrivacyRejection, privacy_filter, privacy_filter_path};
pub use replay::{ReplayMode, ReplayQuarantine, ReplayReport, replay, replay_with_mode};
pub use store::{SCHEMA_VERSION, SqliteVecStatus, Store, VECTOR_INDEX_DIM};
pub use types::{
    AgentKind, CandidateClaim, CandidateWrite, Claim, ClaimConfirmation, ClaimStatus, ClaimWrite,
    ConsolidationReport, ContradictionRecord, EpisodicEvent, EventWrite, ExtractionDecision,
    ExtractionTriggerReason, ExtractorFact, GraphExpansion, PredicateWalk, Provenance,
    RecallFilters, RecallQuery, ReverifyWrite, ScopeWalk, VectorChunk, Verification,
};
