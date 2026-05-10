//! Aver core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

pub mod extractor;
mod privacy;
mod recall;
pub mod retrieval;
mod seed;
mod types;
mod validation;
pub mod vector;

pub use privacy::{PrivacyRejection, privacy_filter, privacy_filter_path};
pub use types::{
    AgentKind, CandidateClaim, CandidateClaimDraft, Claim, ClaimStatus, Community,
    ConsolidationReport, ContradictionRecord, EpisodicEvent, ExtractionDecision,
    ExtractionTriggerReason, GraphDriftSnapshot, GraphExpansion, NewClaim, Observation,
    ObservationDraft, ObservationRecall, ObservationRelevance, Provenance, StorageMode,
    VectorChunk,
};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use recall::{graph_score_for_query_claim, query_tokens_for_recall, recall_token_score};
use rusqlite::{Connection, OptionalExtension, params, types::Type};
use seed::seed_ontology;
use serde::Serialize;
use validation::{
    validate_agent_id, validate_candidate_status_filter, validate_claim_field,
    validate_contradiction_reason, validate_embedding_model, validate_embedding_vector,
    validate_event_field, validate_observation_field, validate_recall_query,
    validate_rejection_reason, validate_top_k, validate_vector_chunk_text,
};

/// Embedded migrations applied in order on every `Store::open`.
/// Each `CREATE` is `IF NOT EXISTS` so re-application is a no-op (ADR-0005).
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init",
        include_str!("../../../migrations/0001_init.sql"),
    ),
    (
        "0002_vector_chunks",
        include_str!("../../../migrations/0002_vector_chunks.sql"),
    ),
    (
        "0003_ontology",
        include_str!("../../../migrations/0003_ontology.sql"),
    ),
    (
        "0004_episodic_candidates",
        include_str!("../../../migrations/0004_episodic_candidates.sql"),
    ),
    (
        "0005_contradictions",
        include_str!("../../../migrations/0005_contradictions.sql"),
    ),
    (
        "0006_observations",
        include_str!("../../../migrations/0006_observations.sql"),
    ),
    (
        "0007_entities",
        include_str!("../../../migrations/0007_entities.sql"),
    ),
    (
        "0008_privacy_rejections",
        include_str!("../../../migrations/0008_privacy_rejections.sql"),
    ),
    (
        "0009_value_range_checks",
        include_str!("../../../migrations/0009_value_range_checks.sql"),
    ),
    (
        "0010_vector_index",
        include_str!("../../../migrations/0010_vector_index.sql"),
    ),
    (
        "0011_ontology_enforcement",
        include_str!("../../../migrations/0011_ontology_enforcement.sql"),
    ),
    (
        "0012_source_refs_json_checks",
        include_str!("../../../migrations/0012_source_refs_json_checks.sql"),
    ),
    (
        "0013_observation_source_event_ids_json_checks",
        include_str!("../../../migrations/0013_observation_source_event_ids_json_checks.sql"),
    ),
    (
        "0014_vector_embedding_json_checks",
        include_str!("../../../migrations/0014_vector_embedding_json_checks.sql"),
    ),
    (
        "0015_vector_embedding_numeric_checks",
        include_str!("../../../migrations/0015_vector_embedding_numeric_checks.sql"),
    ),
    (
        "0016_observation_source_event_ids_integer_checks",
        include_str!("../../../migrations/0016_observation_source_event_ids_integer_checks.sql"),
    ),
    (
        "0017_claim_source_refs_text_checks",
        include_str!("../../../migrations/0017_claim_source_refs_text_checks.sql"),
    ),
    (
        "0018_observation_source_event_ids_nonempty_checks",
        include_str!("../../../migrations/0018_observation_source_event_ids_nonempty_checks.sql"),
    ),
    (
        "0019_claim_source_refs_nonempty_checks",
        include_str!("../../../migrations/0019_claim_source_refs_nonempty_checks.sql"),
    ),
    (
        "0020_claim_source_refs_nonblank_checks",
        include_str!("../../../migrations/0020_claim_source_refs_nonblank_checks.sql"),
    ),
    (
        "0021_observation_source_event_ids_positive_checks",
        include_str!("../../../migrations/0021_observation_source_event_ids_positive_checks.sql"),
    ),
    (
        "0022_vector_embedding_nonempty_checks",
        include_str!("../../../migrations/0022_vector_embedding_nonempty_checks.sql"),
    ),
    (
        "0023_vector_chunk_text_nonblank_checks",
        include_str!("../../../migrations/0023_vector_chunk_text_nonblank_checks.sql"),
    ),
    (
        "0024_vector_chunk_embedding_model_nonblank_checks",
        include_str!("../../../migrations/0024_vector_chunk_embedding_model_nonblank_checks.sql"),
    ),
    (
        "0025_episodic_event_session_nonblank_checks",
        include_str!("../../../migrations/0025_episodic_event_session_nonblank_checks.sql"),
    ),
    (
        "0026_episodic_event_kind_nonblank_checks",
        include_str!("../../../migrations/0026_episodic_event_kind_nonblank_checks.sql"),
    ),
    (
        "0027_episodic_event_source_nonblank_checks",
        include_str!("../../../migrations/0027_episodic_event_source_nonblank_checks.sql"),
    ),
    (
        "0028_episodic_event_agent_id_nonblank_checks",
        include_str!("../../../migrations/0028_episodic_event_agent_id_nonblank_checks.sql"),
    ),
    (
        "0029_episodic_event_agent_id_charset_checks",
        include_str!("../../../migrations/0029_episodic_event_agent_id_charset_checks.sql"),
    ),
    (
        "0030_claim_agent_id_nonblank_checks",
        include_str!("../../../migrations/0030_claim_agent_id_nonblank_checks.sql"),
    ),
    (
        "0031_claim_agent_id_charset_checks",
        include_str!("../../../migrations/0031_claim_agent_id_charset_checks.sql"),
    ),
    (
        "0032_observation_agent_id_nonblank_checks",
        include_str!("../../../migrations/0032_observation_agent_id_nonblank_checks.sql"),
    ),
    (
        "0033_observation_agent_id_charset_checks",
        include_str!("../../../migrations/0033_observation_agent_id_charset_checks.sql"),
    ),
    (
        "0034_observation_session_nonblank_checks",
        include_str!("../../../migrations/0034_observation_session_nonblank_checks.sql"),
    ),
    (
        "0035_observation_content_nonblank_checks",
        include_str!("../../../migrations/0035_observation_content_nonblank_checks.sql"),
    ),
    (
        "0036_observation_derivation_nonblank_checks",
        include_str!("../../../migrations/0036_observation_derivation_nonblank_checks.sql"),
    ),
    (
        "0037_observation_id_nonblank_checks",
        include_str!("../../../migrations/0037_observation_id_nonblank_checks.sql"),
    ),
    (
        "0038_candidate_claim_subject_nonblank_checks",
        include_str!("../../../migrations/0038_candidate_claim_subject_nonblank_checks.sql"),
    ),
    (
        "0039_candidate_claim_predicate_nonblank_checks",
        include_str!("../../../migrations/0039_candidate_claim_predicate_nonblank_checks.sql"),
    ),
    (
        "0040_candidate_claim_object_nonblank_checks",
        include_str!("../../../migrations/0040_candidate_claim_object_nonblank_checks.sql"),
    ),
    (
        "0041_candidate_rejection_reason_checks",
        include_str!("../../../migrations/0041_candidate_rejection_reason_checks.sql"),
    ),
    (
        "0042_candidate_promotion_claim_id_checks",
        include_str!("../../../migrations/0042_candidate_promotion_claim_id_checks.sql"),
    ),
];

/// Canonical embedding dimension for the `vec0` ANN index (ADR-0017
/// §"Dimension binding"). Bound to `nomic-embed-text`, the default model
/// returned by [`vector::VectorIndexConfig::default`]. Changing this value
/// requires a re-index — see ADR-0017 §"Dimension binding".
pub const VECTOR_INDEX_DIM: usize = 768;

/// Register the statically-linked `sqlite-vec` extension as a SQLite
/// auto-extension (ADR-0017). Called exactly once per process; subsequent
/// calls are no-ops. The auto-extension fires on every new `Connection`,
/// so all `Store::open` and `replay` paths get the `vec0` virtual-table
/// module without extra plumbing.
fn ensure_sqlite_vec_registered() {
    use std::sync::Once;
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `sqlite3_vec_init` matches the SQLite auto-extension ABI.
        // `sqlite3_auto_extension` is process-global; the `Once` guard
        // prevents double-registration. The transmute is the idiom used by
        // the `sqlite-vec` crate's own integration test.
        unsafe {
            type RawAutoExt = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;
            rusqlite::ffi::sqlite3_auto_extension(Some(
                std::mem::transmute::<*const (), RawAutoExt>(
                    sqlite_vec::sqlite3_vec_init as *const (),
                ),
            ));
        }
    });
}

/// Local storage for Aver (ADR-0006).
///
/// Layout under `memory_dir`:
///   db.sqlite  — claims, entities, episodes, contradictions
///   log.jsonl  — append-only audit log (ADR-0005, source of truth)
pub struct Store {
    conn: Connection,
    memory_dir: PathBuf,
    log_path: PathBuf,
    event_log_path: PathBuf,
    observation_log_path: PathBuf,
}

/// Log rotation thresholds (ADR-0019 §5).
pub const LOG_ROTATE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const LOG_ROTATE_MAX_LINES: u64 = 500_000;

/// Runtime availability of the bundled `sqlite-vec` extension (ADR-0017).
/// Static linking via the `sqlite-vec` crate makes `Available` the expected
/// state on every supported platform; the `Unavailable` variant exists so
/// sandboxed builds that disable the auto-extension can degrade gracefully
/// to the JSON full-scan recall path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteVecStatus {
    Available,
    Unavailable { reason: String },
}

type ClaimRow = (
    i64,
    String,
    String,
    String,
    String,
    f64,
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
);

struct ClaimWrite<'a> {
    agent_id: &'a str,
    agent_kind: AgentKind,
    provenance: Provenance,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    source: &'a str,
    confidence: f64,
}

pub trait Observer {
    fn observe(&self, events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockObserver {
    drafts: Vec<ObservationDraft>,
}

impl MockObserver {
    pub fn new(drafts: Vec<ObservationDraft>) -> Self {
        Self { drafts }
    }
}

impl Observer for MockObserver {
    fn observe(&self, _events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error> {
        Ok(self.drafts.clone())
    }
}

pub trait GraphStorageAdapter {
    fn mode(&self) -> StorageMode;
    fn detect_communities(&self) -> Result<Vec<Community>, Error>;
}

pub trait ClaimExtractor {
    fn extract(&self, events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockClaimExtractor {
    drafts: Vec<CandidateClaimDraft>,
}

impl MockClaimExtractor {
    pub fn new(drafts: Vec<CandidateClaimDraft>) -> Self {
        Self { drafts }
    }
}

impl ClaimExtractor for MockClaimExtractor {
    fn extract(&self, _events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error> {
        Ok(self.drafts.clone())
    }
}

fn provenance_for_agent_kind(agent_kind: AgentKind) -> Provenance {
    match agent_kind {
        AgentKind::Human => Provenance::UserAsserted,
        AgentKind::DeterministicParser | AgentKind::ExternalTool => Provenance::Extracted,
        AgentKind::Llm => Provenance::Inferred,
    }
}

impl Store {
    /// Open or create a memory store rooted at `memory_dir`.
    /// The directory is created if it does not exist; migrations are applied.
    pub fn open(memory_dir: impl AsRef<Path>) -> Result<Self, Error> {
        // ADR-0017: register the bundled sqlite-vec extension before any
        // Connection is created so the `vec0` module is available to the
        // 0010 migration and every subsequent statement.
        ensure_sqlite_vec_registered();

        let memory_dir = memory_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&memory_dir)?;

        let db_path = memory_dir.join("db.sqlite");
        let log_path = memory_dir.join("log.jsonl");
        let event_log_path = memory_dir.join("events.jsonl");
        let observation_log_path = memory_dir.join("observations.jsonl");

        // ADR-0019 §5: rotation only at session boundaries — check at open.
        // Also recover any half-rotated `log.{N}.jsonl` left by a prior crash.
        finalize_pending_rotations(&memory_dir)?;
        maybe_rotate_log(&memory_dir, &log_path)?;

        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // ADR-0019 §1: raise wal_autocheckpoint from 1000 to 4000 pages.
        conn.pragma_update(None, "wal_autocheckpoint", 4_000)?;

        // ADR-0019 §6: gate migrations on PRAGMA user_version.
        let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let target = MIGRATIONS.len() as i64;
        if current > target {
            return Err(Error::SchemaTooNew {
                found: current,
                supported: target,
            });
        }
        let start = current.max(0) as usize;
        for (_name, sql) in &MIGRATIONS[start..] {
            conn.execute_batch(sql)?;
        }
        conn.pragma_update(None, "user_version", target)?;
        seed_ontology(&conn)?;

        Ok(Self {
            conn,
            memory_dir,
            log_path,
            event_log_path,
            observation_log_path,
        })
    }

    /// Memory directory rooted at the store. Used by CLI tools (vacuum, replay).
    pub fn memory_dir(&self) -> &Path {
        &self.memory_dir
    }

    /// Read the connection-local `wal_autocheckpoint` setting. Inspection helper
    /// for ADR-0019 §1 — the pragma is per-connection and cannot be observed by
    /// reopening a fresh `Connection`.
    pub fn wal_autocheckpoint(&self) -> Result<i64, Error> {
        Ok(self
            .conn
            .pragma_query_value(None, "wal_autocheckpoint", |r| r.get(0))?)
    }

    /// Explicit close: runs `PRAGMA wal_checkpoint(TRUNCATE)` to leave no WAL
    /// behind on clean shutdown (ADR-0019 §1), then drops the connection.
    pub fn close(self) -> Result<(), Error> {
        self.conn
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        drop(self.conn);
        Ok(())
    }

    /// Whether a table with the given name exists. Test/inspection helper.
    pub fn has_table(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [name],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Probe `sqlite-vec` capability. With the bundled crate this is
    /// expected to return `Available` everywhere; the probe is retained
    /// so callers can detect a sandboxed build where the auto-extension
    /// failed to register and fall back to the JSON recall path
    /// (ADR-0017 §"Retrieval rewrite").
    pub fn sqlite_vec_status(&self) -> Result<SqliteVecStatus, Error> {
        let available = self
            .conn
            .query_row("SELECT vec_version()", [], |_| Ok(()))
            .is_ok();
        if available {
            Ok(SqliteVecStatus::Available)
        } else {
            Ok(SqliteVecStatus::Unavailable {
                reason: "sqlite-vec extension is not loaded".to_string(),
            })
        }
    }

    /// Whether the `vec0` virtual table created by migration 0010 exists.
    /// True on every fresh `Store::open`; false only when migrations were
    /// blocked (e.g. extension missing in a sandbox build).
    pub fn vector_index_table_exists(&self) -> Result<bool, Error> {
        Ok(self.has_table("vector_index"))
    }

    /// Number of rows currently in the `vec0` ANN index. Test/inspection
    /// helper for ADR-0017's backfill correctness.
    pub fn vector_index_row_count(&self) -> Result<i64, Error> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM vector_index", [], |r| r.get(0))?)
    }

    pub fn predicate_implies(&self, predicate: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("predicate", ancestor)?;
        if predicate == ancestor {
            return Ok(true);
        }
        let Some(predicate_id) = self.predicate_type_id(predicate)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.predicate_type_id(ancestor)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM predicate_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![predicate_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_is_a(&self, type_name: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("entity_type", type_name)?;
        validate_claim_field("entity_type", ancestor)?;
        if type_name == ancestor {
            return Ok(true);
        }
        let Some(type_id) = self.entity_type_id(type_name)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.entity_type_id(ancestor)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM entity_type_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![type_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_name(&self, entity: &str) -> Result<String, Error> {
        validate_claim_field("entity", entity)?;
        self.conn
            .query_row(
                "SELECT entity_types.name
                   FROM entities
                   JOIN entity_types ON entity_types.id = entities.type_id
                  WHERE entities.name = ?1",
                [entity],
                |row| row.get(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEntity {
                    entity: entity.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn entity_is_a_type(&self, entity: &str, ancestor: &str) -> Result<bool, Error> {
        let type_name = self.entity_type_name(entity)?;
        self.entity_type_is_a(&type_name, ancestor)
    }

    fn expand_predicate_filter(&self, predicates: &[&str]) -> Result<HashSet<String>, Error> {
        let mut allowed = HashSet::new();
        for predicate in predicates {
            allowed.insert((*predicate).to_string());
            let mut stmt = self.conn.prepare(
                "SELECT child.name
                   FROM predicate_types child
                   JOIN predicate_closure closure ON closure.child_id = child.id
                   JOIN predicate_types ancestor ON ancestor.id = closure.ancestor_id
                  WHERE ancestor.name = ?1
                  ORDER BY child.id",
            )?;
            let rows = stmt.query_map([*predicate], |row| row.get::<_, String>(0))?;
            for row in rows {
                allowed.insert(row?);
            }
        }
        Ok(allowed)
    }

    fn ensure_entity(&self, entity: &str, now: i64) -> Result<(), Error> {
        let inferred_type = self.infer_entity_type_name(entity)?;
        let type_id = self.entity_type_id(&inferred_type)?.unwrap_or_else(|| {
            self.entity_type_id("Thing")
                .expect("Thing lookup should not fail")
                .expect("ontology bootstrap should seed Thing")
        });
        let thing_id = self
            .entity_type_id("Thing")?
            .expect("ontology bootstrap should seed Thing");
        // ADR-0018 §"Subject/object policy": when the inferred type falls
        // back to `Thing` (no `prefix:` and no synonym match), surface the
        // entity for consolidation review instead of silently coercing.
        let requires_review = if type_id == thing_id { 1_i64 } else { 0_i64 };
        let current: Option<i64> = self
            .conn
            .query_row(
                "SELECT type_id FROM entities WHERE name = ?1",
                [entity],
                |row| row.get(0),
            )
            .optional()?;
        match current {
            None => {
                self.conn.execute(
                    "INSERT INTO entities (name, type_id, requires_review, created_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![entity, type_id, requires_review, now],
                )?;
            }
            Some(existing) if existing == thing_id && type_id != thing_id => {
                // Promotion from Thing → real type clears the review flag.
                self.conn.execute(
                    "UPDATE entities
                        SET type_id = ?2, requires_review = 0, last_seen_at = ?3
                      WHERE name = ?1",
                    params![entity, type_id, now],
                )?;
            }
            Some(_) => {
                self.conn.execute(
                    "UPDATE entities SET last_seen_at = ?2 WHERE name = ?1",
                    params![entity, now],
                )?;
            }
        }
        Ok(())
    }

    /// ADR-0018: count of entities currently flagged `requires_review = 1`.
    /// Surfaces the silent-`Thing`-fallback queue for consolidation.
    pub fn requires_review_count(&self) -> Result<i64, Error> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE requires_review = 1",
            [],
            |row| row.get(0),
        )?)
    }

    /// ADR-0018: resolve a predicate against `predicate_types.name` and the
    /// `predicate_alias` table.
    ///
    /// Returns `Ok(true)` if the predicate is canonical or aliased; on miss
    /// the policy diverges by provenance:
    ///   * `USER_ASSERTED` — auto-extend `predicate_types` with parent
    ///     `relates_to`, log to `ontology_extension_log`, return `Ok(true)`.
    ///   * everything else — return `Err(Error::UnknownPredicate)`.
    fn ontology_check(
        &self,
        predicate: &str,
        provenance: Provenance,
        agent_id: &str,
        now: i64,
    ) -> Result<(), Error> {
        if self.predicate_type_id(predicate)?.is_some() {
            return Ok(());
        }
        let alias_hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT predicate_id FROM predicate_alias WHERE alias = ?1",
                [predicate],
                |row| row.get(0),
            )
            .optional()?;
        if alias_hit.is_some() {
            return Ok(());
        }
        match provenance {
            Provenance::UserAsserted => {
                let parent_id = self
                    .predicate_type_id("relates_to")?
                    .expect("ontology bootstrap should seed relates_to");
                self.conn.execute(
                    "INSERT INTO predicate_types (name, parent_id, created_via, created_at)
                     VALUES (?1, ?2, 'user_assertion', ?3)",
                    params![predicate, parent_id, now],
                )?;
                // Closure rebuild covers the new id incrementally; the
                // rebuild is cheap (small ontology) and matches the
                // pattern in `seed_ontology`.
                seed::rebuild_closure(&self.conn, "predicate_types", "predicate_closure")?;
                self.conn.execute(
                    "INSERT INTO ontology_extension_log
                       (predicate, parent, agent_id, created_at)
                     VALUES (?1, 'relates_to', ?2, ?3)",
                    params![predicate, agent_id, now],
                )?;
                Ok(())
            }
            Provenance::Extracted | Provenance::Inferred | Provenance::Ambiguous => {
                Err(Error::UnknownPredicate {
                    name: predicate.to_string(),
                })
            }
        }
    }

    fn infer_entity_type_name(&self, entity: &str) -> Result<String, Error> {
        if let Some((prefix, _rest)) = entity.split_once(':')
            && self.entity_type_id(prefix)?.is_some()
        {
            return Ok(prefix.to_string());
        }
        match entity {
            "User" => Ok("Human".to_string()),
            "Claude" | "Pi" => Ok("Bot".to_string()),
            _ => Ok("Thing".to_string()),
        }
    }

    fn entity_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM entity_types WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }

    fn predicate_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM predicate_types WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }

    /// Append a USER_ASSERTED claim. Pre-allocates the claim id, writes
    /// the JSONL log line first (source of truth, ADR-0005), then mirrors
    /// into SQLite with the same explicit id so audit replay can rebuild
    /// the DB from the log without id drift.
    /// Default confidence is 0.95 per ADR-0003's policy table.
    pub fn add_claim(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.add_claim_with_confidence(subject, predicate, object, source, 0.95)
    }

    pub fn add_claim_with_confidence(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        confidence: f64,
    ) -> Result<i64, Error> {
        self.insert_claim(ClaimWrite {
            agent_id: "local",
            agent_kind: AgentKind::Human,
            provenance: Provenance::UserAsserted,
            subject,
            predicate,
            object,
            source,
            confidence,
        })
    }

    pub fn add_claim_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        let provenance = provenance_for_agent_kind(agent_kind);
        self.insert_claim(ClaimWrite {
            agent_id,
            agent_kind,
            provenance,
            subject,
            predicate,
            object,
            source,
            confidence: provenance.policy_confidence(),
        })
    }

    fn insert_claim(&self, write: ClaimWrite<'_>) -> Result<i64, Error> {
        validate_claim_field("subject", write.subject)?;
        validate_claim_field("predicate", write.predicate)?;
        validate_claim_field("object", write.object)?;
        validate_claim_field("source", write.source)?;
        if !(0.0..=1.0).contains(&write.confidence) {
            return Err(Error::InvalidConfidence {
                value: write.confidence,
            });
        }
        validate_agent_id(write.agent_id)?;
        if let Err(rejection) = privacy_filter(&format!(
            "{} {} {} {} {} {}",
            write.agent_id,
            write.agent_kind.as_str(),
            write.subject,
            write.predicate,
            write.object,
            write.source
        )) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        for possible_path in [write.subject, write.object, write.source] {
            self.privacy_filter_path_recording(possible_path)?;
        }

        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // ADR-0018: ontology check. USER_ASSERTED writes auto-extend the
        // ontology with audit trail; EXTRACTED/INFERRED/AMBIGUOUS writes
        // reject unknown predicates. Runs after the privacy filter so a
        // secret-bearing predicate is quarantined first (see ADR-0018
        // §"Telemetry").
        self.ontology_check(write.predicate, write.provenance, write.agent_id, now)?;

        // Pre-allocate the claim id. Single-writer assumption: rusqlite's
        // Connection is !Sync, so within a process this is race-free; SQLite
        // WAL serializes writers across processes.
        let claim_id: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                    r.get(0)
                })?;

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            claim_id,
            subject: write.subject,
            predicate: write.predicate,
            object: write.object,
            source: write.source,
            agent_id: write.agent_id,
            agent_kind: write.agent_kind.as_str(),
            confidence: write.confidence,
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(write.agent_id)?, &entry)?;

        self.ensure_entity(write.subject, now)?;
        self.ensure_entity(write.object, now)?;

        let source_refs = serde_json::to_string(&[write.source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind, write_ts,
                                 created_at, last_seen_at, last_verified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7,
                     ?8, ?9, ?10, ?10, ?10, ?10)",
            params![
                claim_id,
                write.subject,
                write.predicate,
                write.object,
                write.provenance.as_str(),
                write.confidence,
                source_refs,
                write.agent_id,
                write.agent_kind.as_str(),
                now
            ],
        )?;
        Ok(claim_id)
    }

    fn record_privacy_rejection(&self, rejection: PrivacyRejection) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO privacy_rejections (reason, count) VALUES (?1, 1)
             ON CONFLICT(reason) DO UPDATE SET count = count + 1",
            [rejection.telemetry_reason()],
        )?;
        Ok(())
    }

    fn privacy_filter_recording(&self, content: &str) -> Result<(), Error> {
        if let Err(rejection) = privacy_filter(content) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        Ok(())
    }

    pub fn privacy_filter_path_recording(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        if let Err(rejection) = privacy_filter_path(path) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        Ok(())
    }

    pub fn privacy_rejection_count(&self, rejection: PrivacyRejection) -> Result<i64, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT count FROM privacy_rejections WHERE reason = ?1",
                [rejection.telemetry_reason()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    fn agent_log_path(&self, agent_id: &str) -> Result<PathBuf, Error> {
        validate_agent_id(agent_id)?;
        Ok(self
            .log_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("agents")
            .join(agent_id)
            .join("log.jsonl"))
    }

    pub fn record_event(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.record_event_from_agent("local", AgentKind::Human, session_id, kind, payload, source)
    }

    pub fn record_event_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        validate_event_field("session_id", session_id)?;
        validate_event_field("kind", kind)?;
        validate_event_field("source", source)?;
        validate_agent_id(agent_id)?;
        self.privacy_filter_recording(&format!(
            "{agent_id} {} {session_id} {kind} {payload} {source}",
            agent_kind.as_str()
        ))?;
        self.privacy_filter_path_recording(source)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let event_id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM episodic_events",
            [],
            |r| r.get(0),
        )?;
        let entry = EventLogEntry {
            kind: "record_event",
            ts: now,
            event_id,
            session_id,
            event_kind: kind,
            payload,
            source,
            agent_id,
            agent_kind: agent_kind.as_str(),
        };
        append_jsonl(&self.event_log_path, &entry)?;
        self.conn.execute(
            "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event_id,
                session_id,
                kind,
                payload,
                source,
                agent_id,
                agent_kind.as_str(),
                now
            ],
        )?;
        Ok(event_id)
    }

    pub fn get_event(&self, id: i64) -> Result<EpisodicEvent, Error> {
        let (id, session_id, kind, payload, source, agent_id, agent_kind, ts): (
            i64,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
        ) = self
            .conn
            .query_row(
                "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts
               FROM episodic_events WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEvent { event_id: id },
                other => Error::Sqlite(other),
            })?;

        Ok(EpisodicEvent {
            id,
            session_id,
            kind,
            payload,
            source,
            agent_id,
            agent_kind: agent_kind.parse()?,
            ts,
        })
    }

    pub fn propose_claims_from_extractor(
        &self,
        session_id: &str,
        extractor: &impl ClaimExtractor,
    ) -> Result<Vec<i64>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = extractor.extract(&events)?;
        let mut candidate_ids = Vec::new();
        for draft in drafts {
            candidate_ids.push(self.propose_candidate_claim(
                draft.event_id,
                &draft.subject,
                &draft.predicate,
                &draft.object,
            )?);
        }
        Ok(candidate_ids)
    }

    pub fn list_events_for_session(&self, session_id: &str) -> Result<Vec<EpisodicEvent>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM episodic_events WHERE session_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(self.get_event(row?)?);
        }
        Ok(events)
    }

    pub fn record_observation(
        &self,
        session_id: &str,
        content: &str,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: &str,
    ) -> Result<String, Error> {
        validate_event_field("session_id", session_id)?;
        validate_observation_field("content", content)?;
        validate_observation_field("derivation", derivation)?;
        if source_event_ids.is_empty() {
            return Err(Error::MissingEventProvenance { event_id: 0 });
        }
        self.privacy_filter_recording(&format!("{session_id} {content} {derivation}"))?;
        self.privacy_filter_path_recording(derivation)?;

        let mut events = Vec::new();
        for event_id in source_event_ids {
            let event = self.get_event(*event_id)?;
            if event.session_id != session_id {
                return Err(Error::MissingEventProvenance {
                    event_id: *event_id,
                });
            }
            events.push(event);
        }
        let first_event = events
            .first()
            .expect("source_event_ids is checked non-empty before event lookup");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let id = observation_id(session_id, content, source_event_ids);
        let source_event_ids_json = serde_json::to_string(source_event_ids)?;
        let entry = ObservationLogEntry {
            kind: "record_observation",
            ts: now,
            observation_id: &id,
            session_id,
            content,
            relevance: relevance.as_str(),
            source_event_ids,
            agent_id: &first_event.agent_id,
            agent_kind: first_event.agent_kind.as_str(),
            derivation,
        };
        append_jsonl(&self.observation_log_path, &entry)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO observations
             (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                session_id,
                content,
                relevance.as_str(),
                source_event_ids_json,
                first_event.agent_id,
                first_event.agent_kind.as_str(),
                derivation,
                now
            ],
        )?;
        Ok(id)
    }

    pub fn propose_observations_from_observer(
        &self,
        session_id: &str,
        observer: &impl Observer,
    ) -> Result<Vec<String>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = observer.observe(&events)?;
        let mut ids = Vec::new();
        for draft in drafts {
            ids.push(self.record_observation(
                session_id,
                &draft.content,
                draft.relevance,
                &draft.source_event_ids,
                &draft.derivation,
            )?);
        }
        Ok(ids)
    }

    pub fn get_observation(&self, id: &str) -> Result<Observation, Error> {
        validate_observation_field("id", id)?;
        self.conn
            .query_row(
                "SELECT id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts
                   FROM observations WHERE id = ?1",
                [id],
                |row| {
                    let relevance: String = row.get(3)?;
                    let relevance = relevance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err))
                    })?;
                    let source_event_ids_json: String = row.get(4)?;
                    let source_event_ids = serde_json::from_str(&source_event_ids_json).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
                    })?;
                    let agent_kind: String = row.get(6)?;
                    let agent_kind = agent_kind.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err))
                    })?;
                    Ok(Observation {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        content: row.get(2)?,
                        relevance,
                        source_event_ids,
                        agent_id: row.get(5)?,
                        agent_kind,
                        derivation: row.get(7)?,
                        ts: row.get(8)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingObservation {
                    observation_id: id.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn list_observations_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<Observation>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM observations WHERE session_id = ?1 ORDER BY ts, id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut observations = Vec::new();
        for row in rows {
            observations.push(self.get_observation(&row?)?);
        }
        Ok(observations)
    }

    pub fn recall_observation(&self, id: &str) -> Result<ObservationRecall, Error> {
        let observation = self.get_observation(id)?;
        let mut events = Vec::new();
        for event_id in &observation.source_event_ids {
            events.push(self.get_event(*event_id)?);
        }
        Ok(ObservationRecall {
            observation,
            events,
        })
    }

    pub fn assemble_compaction_summary(&self, session_id: &str) -> Result<String, Error> {
        let observations = self.list_observations_for_session(session_id)?;
        let mut summary = String::from("# Aver session continuity summary\n\n");
        if observations.is_empty() {
            summary.push_str("No observations recorded.\n");
            return Ok(summary);
        }
        for observation in observations {
            summary.push_str(&format!(
                "- [{}] {} (id={}, source_events={:?})\n",
                observation.relevance.as_str(),
                observation.content,
                observation.id,
                observation.source_event_ids
            ));
        }
        Ok(summary)
    }

    pub fn prune_observations(&self, session_id: &str, keep: usize) -> Result<usize, Error> {
        validate_event_field("session_id", session_id)?;
        let mut observations = self.list_observations_for_session(session_id)?;
        if observations.len() <= keep {
            return Ok(0);
        }
        observations.sort_by_key(|observation| {
            (
                observation.relevance.rank(),
                observation.ts,
                observation.id.clone(),
            )
        });
        let drop_count = observations.len() - keep;
        for observation in observations.iter().take(drop_count) {
            self.conn
                .execute("DELETE FROM observations WHERE id = ?1", [&observation.id])?;
        }
        Ok(drop_count)
    }

    pub fn graph_drift_snapshot(
        &self,
        consolidation: ConsolidationReport,
    ) -> Result<GraphDriftSnapshot, Error> {
        let mut claim_count_by_provenance = BTreeMap::new();
        let mut mean_confidence_by_provenance = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT provenance, COUNT(*), AVG(confidence)
               FROM claims GROUP BY provenance ORDER BY provenance",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        let mut total_claims = 0_u64;
        let mut ambiguous_claims = 0_u64;
        for row in rows {
            let (provenance, count, mean_confidence) = row?;
            if provenance == Provenance::Ambiguous.as_str() {
                ambiguous_claims = count;
            }
            total_claims += count;
            claim_count_by_provenance.insert(provenance.clone(), count);
            mean_confidence_by_provenance.insert(provenance, mean_confidence);
        }

        let contradicts_edge_count =
            self.conn
                .query_row("SELECT COUNT(*) FROM contradictions", [], |row| {
                    row.get::<_, u64>(0)
                })?;
        let mut entity_count_by_type_id = BTreeMap::new();
        let mut entity_stmt = self.conn.prepare(
            "SELECT entity_types.name, COUNT(entities.name)
               FROM entities JOIN entity_types ON entities.type_id = entity_types.id
              GROUP BY entity_types.name ORDER BY entity_types.name",
        )?;
        let entity_rows = entity_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in entity_rows {
            let (type_id, count) = row?;
            entity_count_by_type_id.insert(type_id, count);
        }

        let mut privacy_rejection_counts = BTreeMap::new();
        let mut privacy_stmt = self
            .conn
            .prepare("SELECT reason, count FROM privacy_rejections ORDER BY reason")?;
        let privacy_rows = privacy_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in privacy_rows {
            let (reason, count) = row?;
            privacy_rejection_counts.insert(reason, count);
        }

        Ok(GraphDriftSnapshot {
            claim_count_by_provenance,
            mean_confidence_by_provenance,
            contradicts_edge_count,
            ambiguous_ratio: if total_claims == 0 {
                0.0
            } else {
                ambiguous_claims as f64 / total_claims as f64
            },
            entity_count_by_type_id,
            consolidation_merged: consolidation.merged,
            consolidation_superseded: consolidation.superseded,
            privacy_rejection_counts,
        })
    }

    pub fn should_extract_memories(
        &self,
        session_id: &str,
        event_threshold: usize,
    ) -> Result<bool, Error> {
        Ok(self
            .extraction_decision(session_id, event_threshold, None)?
            .should_extract)
    }

    pub fn extraction_decision(
        &self,
        session_id: &str,
        event_threshold: usize,
        observation_token_threshold: Option<usize>,
    ) -> Result<ExtractionDecision, Error> {
        validate_event_field("session_id", session_id)?;
        if event_threshold == 0 {
            return Err(Error::InvalidEventThreshold);
        }
        if observation_token_threshold == Some(0) {
            return Err(Error::InvalidEventThreshold);
        }

        let mut reasons = Vec::new();
        for (kind, reason) in [
            (
                "explicit_remember",
                ExtractionTriggerReason::ExplicitRemember,
            ),
            ("session_end", ExtractionTriggerReason::SessionEnd),
            ("correction", ExtractionTriggerReason::Correction),
            ("commit_completed", ExtractionTriggerReason::CommitCompleted),
            ("idle_compaction", ExtractionTriggerReason::IdleCompaction),
        ] {
            if self
                .conn
                .query_row(
                    "SELECT 1 FROM episodic_events
                      WHERE session_id = ?1 AND kind = ?2
                      LIMIT 1",
                    params![session_id, kind],
                    |_| Ok(()),
                )
                .is_ok()
            {
                reasons.push(reason);
            }
        }

        let event_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM episodic_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        if event_count >= event_threshold {
            reasons.push(ExtractionTriggerReason::EventCountThreshold);
        }

        if let Some(threshold) = observation_token_threshold {
            let token_count: usize = self.conn.query_row(
                "SELECT COALESCE(SUM(
                    CASE
                      WHEN TRIM(payload) = '' THEN 0
                      ELSE LENGTH(TRIM(payload)) - LENGTH(REPLACE(TRIM(payload), ' ', '')) + 1
                    END), 0)
                   FROM episodic_events WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )?;
            if token_count >= threshold {
                reasons.push(ExtractionTriggerReason::ObservationTokenThreshold);
            }
        }

        Ok(ExtractionDecision {
            should_extract: !reasons.is_empty(),
            reasons,
        })
    }

    pub fn propose_candidate_claim(
        &self,
        event_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<i64, Error> {
        if !self.event_exists(event_id)? {
            return Err(Error::MissingEventProvenance { event_id });
        }
        validate_claim_field("subject", subject)?;
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("object", object)?;
        self.privacy_filter_recording(&format!("{subject} {predicate} {object}"))?;
        for possible_path in [subject, object] {
            self.privacy_filter_path_recording(possible_path)?;
        }
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, subject, predicate, object, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn event_exists(&self, event_id: i64) -> Result<bool, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM episodic_events WHERE id = ?1",
                [event_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn promote_candidate_claim(&self, candidate_id: i64) -> Result<i64, Error> {
        let candidate = self.get_candidate_claim(candidate_id)?;
        if candidate.status == "REJECTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        if let Some(claim_id) = candidate.promoted_claim_id {
            return Ok(claim_id);
        }
        let event = self.get_event(candidate.event_id)?;
        let source = format!("event:{}", event.id);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let claim_id: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                    r.get(0)
                })?;

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            claim_id,
            subject: &candidate.subject,
            predicate: &candidate.predicate,
            object: &candidate.object,
            source: &source,
            agent_id: &event.agent_id,
            agent_kind: event.agent_kind.as_str(),
            confidence: candidate.confidence,
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(&event.agent_id)?, &entry)?;
        self.ensure_entity(&candidate.subject, now)?;
        self.ensure_entity(&candidate.object, now)?;

        let source_refs = serde_json::to_string(&[source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind, write_ts,
                                 created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7, ?8, ?9, ?10, ?10, ?10)",
            params![
                claim_id,
                candidate.subject,
                candidate.predicate,
                candidate.object,
                candidate.provenance.as_str(),
                candidate.confidence,
                source_refs,
                event.agent_id,
                event.agent_kind.as_str(),
                now
            ],
        )?;
        self.conn.execute(
            "UPDATE candidate_claims
                SET status = 'PROMOTED', promoted_claim_id = ?1
              WHERE id = ?2",
            params![claim_id, candidate_id],
        )?;
        Ok(claim_id)
    }

    pub fn reject_candidate_claim(&self, candidate_id: i64, reason: &str) -> Result<(), Error> {
        let candidate = match self.get_candidate_claim(candidate_id) {
            Ok(candidate) => candidate,
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingCandidate { candidate_id });
            }
            Err(err) => return Err(err),
        };
        if candidate.status == "PROMOTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        validate_rejection_reason(reason)?;
        self.privacy_filter_recording(reason)?;
        let rows_changed = self.conn.execute(
            "UPDATE candidate_claims
                SET status = 'REJECTED', rejection_reason = ?1
              WHERE id = ?2",
            params![reason, candidate_id],
        )?;
        if rows_changed == 0 {
            return Err(Error::MissingCandidate { candidate_id });
        }
        Ok(())
    }

    pub fn list_candidate_claims(
        &self,
        session_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<CandidateClaim>, Error> {
        if let Some(session_id) = session_id {
            validate_event_field("session_id", session_id)?;
        }
        if let Some(status) = status {
            validate_candidate_status_filter(status)?;
        }
        const CANDIDATE_COLUMNS: &str = "candidate_claims.id, candidate_claims.event_id,
            candidate_claims.subject, candidate_claims.predicate, candidate_claims.object,
            candidate_claims.provenance, candidate_claims.confidence, candidate_claims.status,
            candidate_claims.promoted_claim_id, candidate_claims.rejection_reason";
        let (sql, bind_status, bind_session): (String, Option<&str>, Option<&str>) = match (
            status, session_id,
        ) {
            (Some(status), Some(session_id)) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS}
                           FROM candidate_claims
                           JOIN episodic_events ON episodic_events.id = candidate_claims.event_id
                          WHERE candidate_claims.status = ?1 AND episodic_events.session_id = ?2
                          ORDER BY candidate_claims.id"
                ),
                Some(status),
                Some(session_id),
            ),
            (Some(status), None) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS} FROM candidate_claims WHERE status = ?1 ORDER BY id"
                ),
                Some(status),
                None,
            ),
            (None, Some(session_id)) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS}
                           FROM candidate_claims
                           JOIN episodic_events ON episodic_events.id = candidate_claims.event_id
                          WHERE episodic_events.session_id = ?1
                          ORDER BY candidate_claims.id"
                ),
                None,
                Some(session_id),
            ),
            (None, None) => (
                format!("SELECT {CANDIDATE_COLUMNS} FROM candidate_claims ORDER BY id"),
                None,
                None,
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let map_candidate = |row: &rusqlite::Row<'_>| {
            let provenance_str = row.get::<usize, String>(5)?;
            let provenance = Provenance::from_str(&provenance_str).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
            })?;
            Ok(CandidateClaim {
                id: row.get(0)?,
                event_id: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                provenance,
                confidence: row.get(6)?,
                status: row.get(7)?,
                promoted_claim_id: row.get(8)?,
                rejection_reason: row.get(9)?,
            })
        };
        let rows = match (bind_status, bind_session) {
            (Some(status), Some(session_id)) => stmt
                .query_map(params![status, session_id], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(status), None) => stmt
                .query_map([status], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(session_id)) => stmt
                .query_map([session_id], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    /// List candidate claims for a specific event.
    pub fn list_candidate_claims_for_event(
        &self,
        event_id: i64,
    ) -> Result<Vec<CandidateClaim>, Error> {
        if !self.event_exists(event_id)? {
            return Err(Error::MissingEventProvenance { event_id });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, subject, predicate, object, provenance, confidence, status, promoted_claim_id, rejection_reason
             FROM candidate_claims WHERE event_id = ?1",
        )?;
        let rows = stmt.query_map([event_id], |row| {
            let provenance_str = row.get::<usize, String>(5)?;
            let provenance = Provenance::from_str(&provenance_str).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
            })?;
            Ok(CandidateClaim {
                id: row.get(0)?,
                event_id: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                provenance,
                confidence: row.get(6)?,
                status: row.get(7)?,
                promoted_claim_id: row.get(8)?,
                rejection_reason: row.get(9)?,
            })
        })?;
        let mut candidates = Vec::new();
        for row in rows {
            candidates.push(row?);
        }
        Ok(candidates)
    }

    /// Add a contradiction record for a claim.
    pub fn add_contradiction(&self, claim_id: i64, reason: &str) -> Result<i64, Error> {
        validate_claim_field("reason", reason)?;
        self.privacy_filter_recording(reason)?;
        self.ensure_claim_exists(claim_id)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO contradictions (claim_id, reason, created_at)
             VALUES (?1, ?2, ?3)",
            params![claim_id, reason, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_candidate_claim(&self, id: i64) -> Result<CandidateClaim, Error> {
        self.conn
            .query_row(
                "SELECT id, event_id, subject, predicate, object, provenance, confidence, status,
                    promoted_claim_id, rejection_reason
               FROM candidate_claims WHERE id = ?1",
                [id],
                |row| {
                    let provenance: String = row.get(5)?;
                    let provenance = provenance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
                    })?;
                    Ok(CandidateClaim {
                        id: row.get(0)?,
                        event_id: row.get(1)?,
                        subject: row.get(2)?,
                        predicate: row.get(3)?,
                        object: row.get(4)?,
                        provenance,
                        confidence: row.get(6)?,
                        status: row.get(7)?,
                        promoted_claim_id: row.get(8)?,
                        rejection_reason: row.get(9)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::MissingCandidate { candidate_id: id }
                }
                other => Error::Sqlite(other),
            })
    }

    /// Retrieve a claim by id.
    pub fn get_claim(&self, id: i64) -> Result<Claim, Error> {
        let (
            id,
            subject,
            predicate,
            object,
            provenance,
            confidence,
            status,
            source_refs,
            agent_id,
            agent_kind,
            write_ts,
            last_verified_at,
        ): ClaimRow = self
            .conn
            .query_row(
                "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts, last_verified_at
               FROM claims WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                    ))
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingClaim { claim_id: id },
                other => Error::Sqlite(other),
            })?;

        Ok(Claim {
            id,
            subject,
            predicate,
            object,
            provenance: provenance.parse()?,
            confidence,
            status: status.parse()?,
            source_refs: serde_json::from_str(&source_refs)?,
            agent_id,
            agent_kind: agent_kind.parse()?,
            write_ts,
            last_verified_at,
        })
    }

    /// Insert vector chunk metadata for a claim. The actual sqlite-vss index
    /// is wired separately; this table is the durable join point between
    /// claims and embeddings.
    pub fn add_vector_chunk(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![claim_id, text, embedding_model, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert vector chunk metadata with its embedding vector serialized for
    /// deterministic local storage. When the embedding's dimension matches
    /// the canonical [`VECTOR_INDEX_DIM`], the same vector is also written
    /// to the `vec0` ANN index in the same transaction
    /// (ADR-0017 §"Populate strategy"). Off-dimension rows stay only in
    /// `vector_chunks`; recall covers them via the JSON full-scan fallback.
    pub fn add_vector_chunk_with_embedding(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
        embedding: &[f32],
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
        validate_embedding_vector(embedding)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let embedding_json = serde_json::to_string(embedding)?;

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![claim_id, text, embedding_model, embedding_json, now],
        )?;
        let chunk_id = tx.last_insert_rowid();
        if embedding.len() == VECTOR_INDEX_DIM && self.has_table("vector_index") {
            tx.execute(
                "INSERT INTO vector_index(chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, embedding_json],
            )?;
        }
        tx.commit()?;
        Ok(chunk_id)
    }

    fn ensure_claim_exists(&self, claim_id: i64) -> Result<(), Error> {
        match self.get_claim(claim_id) {
            Ok(_) => Ok(()),
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                Err(Error::MissingClaim { claim_id })
            }
            Err(err) => Err(err),
        }
    }

    /// Insert vector chunk metadata using the canonical claim text rendering.
    pub fn add_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        let claim = self.get_claim(claim_id)?;
        self.add_vector_chunk(claim_id, &claim.text(), embedding_model)
    }

    /// Embed the canonical claim text and persist the resulting vector chunk.
    pub fn add_embedded_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
        client: &impl vector::EmbeddingClient,
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        let claim = self.get_claim(claim_id)?;
        let text = claim.text();
        let embedding = client.embed(&text)?;
        self.add_vector_chunk_with_embedding(claim_id, &text, embedding_model, &embedding)
    }

    /// Retrieve vector chunk metadata by id.
    pub fn get_vector_chunk(&self, id: i64) -> Result<VectorChunk, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, text, embedding_model, embedding_json FROM vector_chunks WHERE id = ?1",
                [id],
                |row| {
                    let embedding_json: Option<String> = row.get(4)?;
                    Ok(VectorChunk {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        text: row.get(2)?,
                        embedding_model: row.get(3)?,
                        embedding: parse_optional_embedding(embedding_json)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingVectorChunk { chunk_id: id },
                other => Error::Sqlite(other),
            })
    }

    /// List vector chunk metadata for a claim in stable insertion order.
    pub fn list_vector_chunks_for_claim(&self, claim_id: i64) -> Result<Vec<VectorChunk>, Error> {
        self.ensure_claim_exists(claim_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let rows = stmt.query_map([claim_id], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut chunks = Vec::new();
        for row in rows {
            chunks.push(row?);
        }
        Ok(chunks)
    }

    /// Returns how many vector_chunks have non-null embeddings vs total.
    pub fn vector_chunk_embedding_status(&self) -> Result<(usize, usize), Error> {
        let total: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM vector_chunks", [], |row| row.get(0))?;
        let indexed: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM vector_chunks WHERE embedding_json IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok((indexed, total))
    }

    /// Backfill stored embeddings for any vector_chunks that have no embedding yet,
    /// using the provided EmbeddingClient. Returns how many were backfilled.
    pub fn backfill_vector_embeddings(
        &self,
        client: &dyn crate::vector::EmbeddingClient,
    ) -> Result<usize, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text FROM vector_chunks WHERE embedding_json IS NULL")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut count = 0;
        let index_present = self.has_table("vector_index");
        for (id, text) in rows {
            let embedding = client.embed(&text)?;
            let embedding_json = serde_json::to_string(&embedding)?;
            self.conn.execute(
                "UPDATE vector_chunks SET embedding_json = ?1 WHERE id = ?2",
                params![embedding_json, id],
            )?;
            // ADR-0017: keep `vector_index` in sync for matching-dim rows.
            // `INSERT OR IGNORE` makes the call idempotent if the row was
            // already backfilled by migration 0010.
            if index_present && embedding.len() == VECTOR_INDEX_DIM {
                self.conn.execute(
                    "INSERT OR IGNORE INTO vector_index(chunk_id, embedding) VALUES (?1, ?2)",
                    params![id, embedding_json],
                )?;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Recall claims ranked by cosine similarity to the query embedding,
    /// combined with text-search results (best score per claim_id wins).
    pub fn recall_text_with_embedding(
        &self,
        query: &str,
        client: &dyn crate::vector::EmbeddingClient,
    ) -> Result<Vec<Claim>, Error> {
        let query_embedding = client.embed(query)?;

        // Score each claim that has a stored embedding.
        let mut scores: HashMap<i64, f64> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT claim_id, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (claim_id, embedding_json) = row?;
            if let Some(embedding) = parse_optional_embedding(embedding_json)?
                && let Some(score) = vector::normalized_cosine_score(&query_embedding, &embedding)
            {
                scores
                    .entry(claim_id)
                    .and_modify(|current| *current = current.max(f64::from(score)))
                    .or_insert(f64::from(score));
            }
        }
        drop(stmt);

        // Merge with text-search results.
        let text_claims = self.recall_text(query).unwrap_or_default();
        let text_score_base = 0.5_f64;
        for claim in &text_claims {
            scores
                .entry(claim.id)
                .and_modify(|current| *current = current.max(text_score_base))
                .or_insert(text_score_base);
        }

        let mut candidates: Vec<(f64, Claim)> = scores
            .keys()
            .copied()
            .filter_map(|claim_id| {
                self.get_claim(claim_id)
                    .ok()
                    .filter(|c| c.status == ClaimStatus::Active)
                    .map(|c| (scores[&claim_id], c))
            })
            .collect();
        candidates.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        Ok(candidates.into_iter().map(|(_, c)| c).collect())
    }

    /// Rank persisted vector chunks by normalized cosine similarity to the
    /// query embedding, returning the best matches first.
    pub fn rank_vector_chunks_by_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        validate_top_k(top_k)?;
        validate_embedding_vector(query_embedding)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut scored = Vec::new();
        for row in rows {
            let chunk = row?;
            if let Some(embedding) = &chunk.embedding
                && let Some(score) = vector::normalized_cosine_score(query_embedding, embedding)
            {
                scored.push((score, chunk));
            }
        }
        scored.sort_by(|(a_score, a_chunk), (b_score, b_chunk)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_chunk.id.cmp(&b_chunk.id))
        });
        scored.truncate(top_k);
        Ok(scored.into_iter().map(|(_, chunk)| chunk).collect())
    }

    /// Embed a query with the provided client, then rank persisted vector
    /// chunks by similarity. Tests use `MockEmbeddingClient`; production can
    /// pass `OllamaEmbeddingClient` without changing storage logic.
    pub fn recall_vector_chunks(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;

        let query_embedding = client.embed(query)?;
        self.rank_vector_chunks_by_embedding(&query_embedding, top_k)
    }

    /// Vector recall that returns claim rows instead of internal chunk
    /// metadata, preserving the chunk ranking order.
    pub fn recall_vector_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let chunks = self.recall_vector_chunks(query, client, usize::MAX)?;
        let mut seen = HashSet::new();
        let mut claims = Vec::new();
        for chunk in chunks {
            if seen.insert(chunk.claim_id) {
                let claim = self.get_claim(chunk.claim_id)?;
                if claim.status != ClaimStatus::Active {
                    continue;
                }
                claims.push(claim);
                if claims.len() == top_k {
                    break;
                }
            }
        }
        Ok(claims)
    }

    /// Hybrid recall over vector chunks plus text fallback. Vector-ranked
    /// claims are returned first; text recall fills any remaining slots with
    /// distinct claims so sparse vector indexes remain useful.
    pub fn recall_hybrid_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        self.recall_hybrid_claims_with_alpha(
            query,
            client,
            top_k,
            retrieval::HybridWeights::for_query(query),
        )
    }

    pub fn recall_hybrid_claims_with_alpha(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
        weights: retrieval::HybridWeights,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;
        let query_embedding = client.embed(query)?;

        // ADR-0017: vector half. When the query embedding matches the
        // canonical dimension and the `vec0` index is present, run a single
        // KNN MATCH instead of an O(N) scan + per-row cosine in Rust. The
        // fanout (`top_k * 4`) gives the graph half room to re-rank without
        // starving on vector-only candidates. Distance is L2 over (assumed)
        // L2-normalised embeddings; we map to a similarity score in [0, 1]
        // via `1 - distance / 2` then clamp, matching the cosine-derived
        // range produced by `normalized_cosine_score`. Off-dimension queries
        // and unindexed installs fall back to the JSON full-scan path.
        let mut vector_scores: HashMap<i64, f64> = HashMap::new();
        let use_vec_index =
            query_embedding.len() == VECTOR_INDEX_DIM && self.has_table("vector_index");
        if use_vec_index {
            let fanout = top_k.saturating_mul(4).max(top_k);
            let q_json = serde_json::to_string(&query_embedding)?;
            let mut stmt = self.conn.prepare(
                "SELECT vc.claim_id, vi.distance
                   FROM vector_index vi
                   JOIN vector_chunks vc ON vc.id = vi.chunk_id
                  WHERE vi.embedding MATCH ?1
                    AND k = ?2
                  ORDER BY vi.distance",
            )?;
            let rows = stmt.query_map(params![q_json, fanout as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (claim_id, distance) = row?;
                let similarity = (1.0 - distance / 2.0).clamp(0.0, 1.0);
                vector_scores
                    .entry(claim_id)
                    .and_modify(|current| *current = current.max(similarity))
                    .or_insert(similarity);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT claim_id, embedding_json
                   FROM vector_chunks
                  WHERE embedding_json IS NOT NULL
                  ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (claim_id, embedding_json) = row?;
                if let Some(embedding) = parse_optional_embedding(embedding_json)?
                    && let Some(score) =
                        vector::normalized_cosine_score(&query_embedding, &embedding)
                {
                    vector_scores
                        .entry(claim_id)
                        .and_modify(|current: &mut f64| *current = current.max(f64::from(score)))
                        .or_insert(f64::from(score));
                }
            }
        }

        let text_claims = self.recall_text(query).unwrap_or_default();
        let mut candidate_ids: HashSet<i64> = text_claims.iter().map(|claim| claim.id).collect();
        candidate_ids.extend(vector_scores.keys().copied());

        let mut candidates = Vec::new();
        for claim_id in candidate_ids {
            let claim = self.get_claim(claim_id)?;
            if claim.status != ClaimStatus::Active {
                continue;
            }
            let vector_score = vector_scores.get(&claim_id).copied().unwrap_or(0.0);
            let graph_score = graph_score_for_query_claim(query, &claim);
            candidates.push((weights.blend(vector_score, graph_score), claim));
        }
        candidates.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        candidates.truncate(top_k);
        Ok(candidates.into_iter().map(|(_, claim)| claim).collect())
    }

    /// Text-only keyword recall over active claims. This is the v0.1
    /// precursor to HybridRAG: cheap SQLite substring matching across the
    /// claim triple fields, ordered deterministically by id.
    pub fn expand(
        &self,
        entity: &str,
        hops: usize,
        predicates: Option<&[&str]>,
    ) -> Result<GraphExpansion, Error> {
        if entity.trim().is_empty() {
            return Err(Error::InvalidGraphEntity);
        }
        if hops == 0 {
            return Err(Error::InvalidGraphHops);
        }

        let predicate_filter = if let Some(items) = predicates {
            if items.is_empty() || items.iter().any(|item| item.trim().is_empty()) {
                return Err(Error::InvalidPredicateFilter);
            }
            Some(self.expand_predicate_filter(items)?)
        } else {
            None
        };
        let mut nodes = vec![entity.to_string()];
        let mut seen_nodes = HashSet::from([entity.to_string()]);
        let mut seen_edges = HashSet::new();
        let mut queue = VecDeque::from([(entity.to_string(), 0usize)]);
        let mut edges = Vec::new();

        while let Some((current, depth)) = queue.pop_front() {
            if depth == hops {
                continue;
            }
            for claim in self.active_claim_edges_for_entity(&current)? {
                if predicate_filter
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(claim.predicate.as_str()))
                {
                    continue;
                }
                if seen_edges.insert(claim.id) {
                    for node in [&claim.subject, &claim.object] {
                        if seen_nodes.insert(node.clone()) {
                            nodes.push(node.clone());
                            queue.push_back((node.clone(), depth + 1));
                        }
                    }
                    edges.push(claim);
                }
            }
        }

        Ok(GraphExpansion { nodes, edges })
    }

    fn active_claim_edges_for_entity(&self, entity: &str) -> Result<Vec<Claim>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
                AND (subject = ?1 OR object = ?1)
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([entity], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.get_claim(id)).collect()
    }

    pub fn detect_communities(&self) -> Result<Vec<Community>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
              ORDER BY id",
        )?;
        let claims = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|id| self.get_claim(id))
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen_nodes = HashSet::new();
        let mut communities = Vec::new();
        for claim in &claims {
            if seen_nodes.contains(&claim.subject) && seen_nodes.contains(&claim.object) {
                continue;
            }
            let graph = self.expand(&claim.subject, usize::MAX, None)?;
            let mut members: Vec<String> = graph
                .nodes
                .into_iter()
                .filter(|node| seen_nodes.insert(node.clone()))
                .collect();
            if !members.is_empty() {
                members.sort();
                let id = format!("community:{}", members.join("-"));
                communities.push(Community { id, members });
            }
        }
        communities.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(communities)
    }

    pub fn agent_trust_score(&self, agent_id: &str) -> Result<f64, Error> {
        validate_agent_id(agent_id)?;
        let (active, total): (i64, i64) = self.conn.query_row(
            "SELECT
                 SUM(CASE WHEN status = 'ACTIVE' THEN 1 ELSE 0 END),
                 COUNT(*)
               FROM claims
              WHERE agent_id = ?1",
            [agent_id],
            |row| Ok((row.get::<_, Option<i64>>(0)?.unwrap_or(0), row.get(1)?)),
        )?;
        if total == 0 {
            return Ok(0.5);
        }
        Ok(((active as f64) / (total as f64)).clamp(0.1, 1.0))
    }

    pub fn contradict(
        &self,
        claim_id: i64,
        reason: &str,
        new_claim: Option<NewClaim<'_>>,
    ) -> Result<ContradictionRecord, Error> {
        match self.get_claim(claim_id) {
            Ok(_) => {}
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingClaim { claim_id });
            }
            Err(err) => return Err(err),
        }
        validate_contradiction_reason(reason)?;
        self.privacy_filter_recording(reason)?;
        let new_claim_id = if let Some(claim) = new_claim {
            Some(self.add_claim(claim.subject, claim.predicate, claim.object, claim.source)?)
        } else {
            None
        };
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO contradictions (claim_id, reason, new_claim_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![claim_id, reason, new_claim_id, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_contradiction(id)
    }

    pub fn list_contradictions(&self, claim_id: i64) -> Result<Vec<ContradictionRecord>, Error> {
        match self.get_claim(claim_id) {
            Ok(_) => {}
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingClaim { claim_id });
            }
            Err(err) => return Err(err),
        }
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM contradictions
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([claim_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.get_contradiction(id))
            .collect()
    }

    fn get_contradiction(&self, id: i64) -> Result<ContradictionRecord, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, reason, new_claim_id, status, created_at
               FROM contradictions
              WHERE id = ?1",
                [id],
                |row| {
                    Ok(ContradictionRecord {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        reason: row.get(2)?,
                        new_claim_id: row.get(3)?,
                        status: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    pub fn should_merge_synonym(similarity: f64) -> bool {
        similarity >= 0.92
    }

    pub fn decay_contradicted_confidence(&self) -> Result<usize, Error> {
        Ok(self.conn.execute(
            "UPDATE claims
                SET confidence = MAX(0.0, ROUND(confidence - 0.10, 2))
              WHERE status = 'ACTIVE'
                AND EXISTS (
                    SELECT 1
                      FROM contradictions
                     WHERE contradictions.claim_id = claims.id
                       AND contradictions.status = 'RECORDED'
                )",
            [],
        )?)
    }

    pub fn decay_inferred_confidence_at(
        &self,
        now_ts: i64,
        tau_seconds: f64,
    ) -> Result<usize, Error> {
        if tau_seconds <= 0.0 || !tau_seconds.is_finite() {
            return Err(Error::InvalidDecayTau { value: tau_seconds });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, confidence, last_seen_at
               FROM claims
              WHERE status = 'ACTIVE' AND provenance = 'INFERRED'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut changed = 0;
        for (id, confidence, last_seen_at) in rows {
            let delta = now_ts.saturating_sub(last_seen_at) as f64;
            let decayed = confidence * (-delta / tau_seconds).exp();
            self.conn.execute(
                "UPDATE claims SET confidence = ?1 WHERE id = ?2",
                params![decayed, id],
            )?;
            changed += 1;
        }
        Ok(changed)
    }

    pub fn consolidate(&self) -> Result<usize, Error> {
        Ok(self.consolidate_report()?.superseded)
    }

    pub fn consolidate_report(&self) -> Result<ConsolidationReport, Error> {
        let merged = self.merge_duplicate_source_refs()?;
        let decayed = self.decay_contradicted_confidence()?;
        let duplicate_changed = self.conn.execute(
            "UPDATE claims
                SET status = 'SUPERSEDED'
              WHERE id NOT IN (
                    SELECT MIN(id)
                      FROM claims
                     GROUP BY subject, predicate, object
              )
                AND status = 'ACTIVE'",
            [],
        )?;
        let conflict_changed = self.conn.execute(
            "UPDATE claims
                SET status = 'SUPERSEDED'
              WHERE status = 'ACTIVE'
                AND EXISTS (
                    SELECT 1
                      FROM claims newer
                     WHERE newer.subject = claims.subject
                       AND newer.predicate = claims.predicate
                       AND newer.object <> claims.object
                       AND newer.id > claims.id
                )",
            [],
        )?;
        Ok(ConsolidationReport {
            merged,
            superseded: duplicate_changed + conflict_changed,
            decayed,
        })
    }

    fn merge_duplicate_source_refs(&self) -> Result<usize, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT subject, predicate, object, MIN(id)
               FROM claims
              GROUP BY subject, predicate, object
             HAVING COUNT(*) > 1",
        )?;
        let groups = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut merged_groups = 0;
        for (subject, predicate, object, survivor_id) in groups {
            let mut source_refs = Vec::new();
            let mut refs_stmt = self.conn.prepare(
                "SELECT source_refs
                   FROM claims
                  WHERE subject = ?1 AND predicate = ?2 AND object = ?3
                  ORDER BY id",
            )?;
            let refs_rows = refs_stmt.query_map(params![subject, predicate, object], |row| {
                row.get::<_, String>(0)
            })?;
            for refs_json in refs_rows {
                for source_ref in serde_json::from_str::<Vec<String>>(&refs_json?)? {
                    if !source_refs.contains(&source_ref) {
                        source_refs.push(source_ref);
                    }
                }
            }
            let merged = serde_json::to_string(&source_refs)?;
            let survivor = self.get_claim(survivor_id)?;
            let should_promote =
                survivor.provenance == Provenance::Inferred && source_refs.len() >= 2;
            if should_promote {
                self.conn.execute(
                    "UPDATE claims
                        SET source_refs = ?1, provenance = 'EXTRACTED', confidence = MAX(confidence, 0.75)
                      WHERE id = ?2",
                    params![merged, survivor_id],
                )?;
            } else {
                self.conn.execute(
                    "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
                    params![merged, survivor_id],
                )?;
            }
            merged_groups += 1;
        }
        Ok(merged_groups)
    }

    pub fn recall_text(&self, query: &str) -> Result<Vec<Claim>, Error> {
        let query_tokens = query_tokens_for_recall(query);
        if query_tokens.is_empty() {
            return Err(Error::InvalidRecallQuery);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts, last_verified_at
               FROM claims
              WHERE status = 'ACTIVE'
              ORDER BY id",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, Option<i64>>(11)?,
            ))
        })?;

        let mut scored_claims = Vec::new();
        for row in rows {
            let (
                id,
                subject,
                predicate,
                object,
                provenance,
                confidence,
                status,
                source_refs,
                agent_id,
                agent_kind,
                write_ts,
                last_verified_at,
            ) = row?;
            let claim = Claim {
                id,
                subject,
                predicate,
                object,
                provenance: provenance.parse()?,
                confidence,
                status: status.parse()?,
                source_refs: serde_json::from_str(&source_refs)?,
                agent_id,
                agent_kind: agent_kind.parse()?,
                write_ts,
                last_verified_at,
            };
            let score = recall_token_score(&query_tokens, &claim);
            if score > 0 {
                scored_claims.push((score, claim));
            }
        }
        let max_score = scored_claims
            .iter()
            .map(|(score, _)| *score)
            .max()
            .unwrap_or(0);
        let name_anchor_subject = if query_tokens.len() == 2 && max_score >= 4 {
            scored_claims
                .iter()
                .find(|(score, claim)| {
                    *score == max_score && claim.predicate.eq_ignore_ascii_case("name")
                })
                .map(|(_, claim)| claim.subject.clone())
        } else {
            None
        };
        let has_name_anchor = name_anchor_subject.is_some();
        let minimum_score = if has_name_anchor {
            2
        } else if max_score >= 5 {
            max_score - 1
        } else if max_score >= 3 {
            3
        } else if max_score >= 2 {
            2
        } else {
            1
        };
        scored_claims.retain(|(score, claim)| {
            *score >= minimum_score
                && name_anchor_subject
                    .as_ref()
                    .is_none_or(|subject| *score == max_score || claim.subject == *subject)
        });
        scored_claims.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        Ok(scored_claims.into_iter().map(|(_, claim)| claim).collect())
    }
}

impl GraphStorageAdapter for Store {
    fn mode(&self) -> StorageMode {
        StorageMode::Local
    }

    fn detect_communities(&self) -> Result<Vec<Community>, Error> {
        Store::detect_communities(self)
    }
}

fn parse_optional_embedding(value: Option<String>) -> rusqlite::Result<Option<Vec<f32>>> {
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
            })
        })
        .transpose()
}

#[derive(Serialize)]
struct EventLogEntry<'a> {
    kind: &'a str,
    ts: i64,
    event_id: i64,
    session_id: &'a str,
    event_kind: &'a str,
    payload: &'a str,
    source: &'a str,
    agent_id: &'a str,
    agent_kind: &'a str,
}

#[derive(Serialize)]
struct ObservationLogEntry<'a> {
    kind: &'a str,
    ts: i64,
    observation_id: &'a str,
    session_id: &'a str,
    content: &'a str,
    relevance: &'a str,
    source_event_ids: &'a [i64],
    agent_id: &'a str,
    agent_kind: &'a str,
    derivation: &'a str,
}

#[derive(Serialize)]
struct LogEntry<'a> {
    kind: &'a str,
    ts: i64,
    claim_id: i64,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    source: &'a str,
    agent_id: &'a str,
    agent_kind: &'a str,
    confidence: f64,
}

fn observation_id(session_id: &str, content: &str, source_event_ids: &[i64]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in session_id
        .as_bytes()
        .iter()
        .chain([0xff].iter())
        .chain(content.as_bytes().iter())
        .chain([0xfe].iter())
        .chain(
            source_event_ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .collect::<Vec<_>>()
                .iter(),
        )
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")[..12].to_string()
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

/// Acquire the advisory `.aver/.lock` PID file (ADR-0019 §2/§5).
/// Returns a guard whose drop releases the lock by deleting the file.
pub struct AverLock {
    path: PathBuf,
}

impl AverLock {
    /// Acquire `<memory_dir>/.lock`. Refuses if a live PID already holds it.
    pub fn acquire(memory_dir: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(memory_dir)?;
        let path = memory_dir.join(".lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let pid = std::process::id();
                writeln!(file, "{pid}")?;
                file.sync_data()?;
                Ok(Self { path })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                // Stale-lock recovery: if the recorded PID is not alive, take it.
                let contents = std::fs::read_to_string(&path).unwrap_or_default();
                let pid: Option<u32> = contents.trim().parse().ok();
                let alive = match pid {
                    Some(p) => process_alive(p),
                    None => false,
                };
                if alive {
                    Err(Error::LockHeld {
                        path: path.display().to_string(),
                    })
                } else {
                    std::fs::remove_file(&path)?;
                    Self::acquire(memory_dir)
                }
            }
            Err(err) => Err(Error::Io(err)),
        }
    }
}

impl Drop for AverLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // signal 0 is "check existence" semantics on POSIX.
    unsafe { libc_kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // Conservative fallback: assume the lock holder is alive.
    true
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// Count newline-terminated lines in a file without loading it whole.
fn count_lines(path: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    let mut lines = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for byte in &buf[..n] {
            if *byte == b'\n' {
                lines += 1;
            }
        }
    }
    Ok(lines)
}

/// Determine the next rotation index `N` for `log.{N}.jsonl[.gz]`.
fn next_rotation_index(memory_dir: &Path) -> std::io::Result<u32> {
    let mut max = 0u32;
    let read_dir = match std::fs::read_dir(memory_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(1),
        Err(err) => return Err(err),
    };
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Match log.{N}.jsonl or log.{N}.jsonl.gz
        let rest = match name.strip_prefix("log.") {
            Some(r) => r,
            None => continue,
        };
        let rest = match rest.strip_suffix(".jsonl.gz") {
            Some(r) => r,
            None => match rest.strip_suffix(".jsonl") {
                Some(r) => r,
                None => continue,
            },
        };
        if rest.is_empty() {
            continue;
        }
        if let Ok(n) = rest.parse::<u32>()
            && n > max
        {
            max = n;
        }
    }
    Ok(max + 1)
}

/// Recover from a partial rotation: any `log.{N}.jsonl` without a matching
/// `.gz` finishes gzipping. ADR-0019 §5.
fn finalize_pending_rotations(memory_dir: &Path) -> Result<(), Error> {
    let read_dir = match std::fs::read_dir(memory_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    let mut pending = Vec::new();
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix("log.") else {
            continue;
        };
        let Some(num) = rest.strip_suffix(".jsonl") else {
            continue;
        };
        if num.is_empty() {
            continue;
        }
        if num.parse::<u32>().is_ok() {
            pending.push(entry.path());
        }
    }
    for src in pending {
        let dst = src.with_extension("jsonl.gz");
        // If the gz already exists, prefer it and remove the orphan plain file.
        if dst.exists() {
            std::fs::remove_file(&src)?;
            continue;
        }
        gzip_file(&src, &dst)?;
        std::fs::remove_file(&src)?;
    }
    Ok(())
}

fn gzip_file(src: &Path, dst: &Path) -> Result<(), Error> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{BufReader, Read};
    let input = std::fs::File::open(src)?;
    let mut input = BufReader::new(input);
    let output = std::fs::File::create(dst)?;
    let mut encoder = GzEncoder::new(output, Compression::default());
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        encoder.write_all(&buf[..n])?;
    }
    let output = encoder.finish()?;
    output.sync_data()?;
    Ok(())
}

/// Rotate `log.jsonl` if it exceeds either size or line threshold.
/// Runs only at session boundaries (called from `Store::open`). ADR-0019 §5.
fn maybe_rotate_log(memory_dir: &Path, log_path: &Path) -> Result<(), Error> {
    let metadata = match std::fs::metadata(log_path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    let size_over = metadata.len() >= LOG_ROTATE_MAX_BYTES;
    let line_over = if size_over {
        true
    } else {
        count_lines(log_path)? >= LOG_ROTATE_MAX_LINES
    };
    if !(size_over || line_over) {
        return Ok(());
    }
    let _lock = AverLock::acquire(memory_dir)?;
    let n = next_rotation_index(memory_dir)?;
    let intermediate = memory_dir.join(format!("log.{n}.jsonl"));
    std::fs::rename(log_path, &intermediate)?;
    let target = memory_dir.join(format!("log.{n}.jsonl.gz"));
    gzip_file(&intermediate, &target)?;
    std::fs::remove_file(&intermediate)?;
    // Touch a fresh empty active log.
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    Ok(())
}

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
    #[error("privacy filter rejected content: {0:?}")]
    Privacy(#[from] PrivacyRejection),
    #[error("invalid {kind} value in database: {value:?}")]
    EnumParse { kind: &'static str, value: String },
    #[error("invalid agent_id for partitioned log path: {value:?}")]
    InvalidAgentId { value: String },
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
    #[error("invalid candidate claim status for candidate {candidate_id}: {status}")]
    InvalidCandidateStatus { candidate_id: i64, status: String },
    #[error("invalid candidate status filter: {status}")]
    InvalidCandidateStatusFilter { status: String },
    #[error("missing entity: {entity}")]
    MissingEntity { entity: String },
    #[error("unknown predicate: {name} (not in predicate_types or predicate_alias)")]
    UnknownPredicate { name: String },
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

/// Stats reported by `aver vacuum`. ADR-0019 §2.
#[derive(Debug, Clone)]
pub struct VacuumReport {
    pub pages_before: i64,
    pub freelist_before: i64,
    pub pages_after: i64,
    pub freelist_after: i64,
    pub vacuumed_into: Option<PathBuf>,
}

/// Run `VACUUM` (or `VACUUM INTO`) plus optional `ANALYZE` against an Aver
/// memory directory. Acquires the advisory lock for the duration. ADR-0019 §2.
pub fn vacuum(
    memory_dir: &Path,
    into: Option<&Path>,
    analyze: bool,
) -> Result<VacuumReport, Error> {
    // ADR-0017: VACUUM rewrites the whole database, including the `vec0`
    // virtual table (whose shadow tables exist as ordinary SQLite tables).
    // The extension must be loaded for SQLite to know about the module.
    ensure_sqlite_vec_registered();

    let _lock = AverLock::acquire(memory_dir)?;
    let db_path = memory_dir.join("db.sqlite");
    let conn = Connection::open(&db_path)?;
    let pages_before: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let freelist_before: i64 = conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;

    let vacuumed_into = if let Some(path) = into {
        // VACUUM INTO 'path' — does not block readers on origin.
        let path_str = path.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{path_str}'"))?;
        Some(path.to_path_buf())
    } else {
        conn.execute_batch("VACUUM")?;
        conn.execute_batch("PRAGMA optimize")?;
        None
    };
    if analyze {
        conn.execute_batch("ANALYZE")?;
    }

    let pages_after: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let freelist_after: i64 = conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;

    Ok(VacuumReport {
        pages_before,
        freelist_before,
        pages_after,
        freelist_after,
        vacuumed_into,
    })
}

/// Stats reported by `aver replay`. ADR-0019 §4.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    pub claims: u64,
    pub events: u64,
    pub observations: u64,
    pub files_walked: u64,
}

/// Replay logs in the deterministic order specified by ADR-0019 §4:
/// rotated `log.{N}.jsonl.gz` (numeric ascending) → `log.jsonl` →
/// `events.jsonl` → `observations.jsonl` → `agents/<id>/log.jsonl`
/// (lexicographic by agent id). Per-agent log records duplicate the global
/// log; replay treats matching content as idempotent and only errors on
/// genuine id-with-different-content collisions.
///
/// Replay BYPASSES the privacy filter — the log is presumed already filtered
/// at write time (ADR-0019 §4).
pub fn replay(memory_dir: &Path, force: bool) -> Result<ReplayReport, Error> {
    use std::io::BufRead;

    // ADR-0017: replay creates a fresh DB and re-runs migrations, including
    // the 0010 vec0 virtual table. The extension must be registered before
    // the partial Connection is opened.
    ensure_sqlite_vec_registered();

    std::fs::create_dir_all(memory_dir)?;
    let db_path = memory_dir.join("db.sqlite");
    let partial_path = memory_dir.join("db.sqlite.partial");

    if db_path.exists() && !force {
        // Refuse if claims is non-empty (per ADR contract).
        let existing = Connection::open(&db_path)?;
        let claim_count: i64 = existing
            .query_row("SELECT COUNT(*) FROM claims", [], |r| r.get(0))
            .unwrap_or(0);
        if claim_count > 0 {
            return Err(Error::ReplayDuplicateId {
                detail: format!(
                    "db.sqlite at {} already has {} claims; pass --force to overwrite",
                    db_path.display(),
                    claim_count
                ),
            });
        }
    }

    // Build the partial db from scratch.
    let _ = std::fs::remove_file(&partial_path);
    let result = (|| -> Result<ReplayReport, Error> {
        let conn = Connection::open(&partial_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "wal_autocheckpoint", 4_000)?;
        for (_name, sql) in MIGRATIONS {
            conn.execute_batch(sql)?;
        }
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        seed_ontology(&conn)?;

        let mut report = ReplayReport::default();
        let inputs = collect_replay_inputs(memory_dir)?;
        for input in inputs {
            report.files_walked += 1;
            let reader: Box<dyn BufRead> = open_log_reader(&input)?;
            for (lineno, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                apply_log_line(
                    &conn,
                    &line,
                    &input.to_string_lossy(),
                    lineno + 1,
                    &mut report,
                )?;
            }
        }

        conn.execute_batch("PRAGMA optimize")?;
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(report)
    })();

    match result {
        Ok(report) => {
            // Atomic swap: only overwrite db.sqtilte after success.
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(memory_dir.join("db.sqlite-wal"));
            let _ = std::fs::remove_file(memory_dir.join("db.sqlite-shm"));
            std::fs::rename(&partial_path, &db_path)?;
            Ok(report)
        }
        Err(err) => Err(err),
    }
}

fn collect_replay_inputs(memory_dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut rotated: Vec<(u32, PathBuf)> = Vec::new();
    let read_dir = match std::fs::read_dir(memory_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(err) => return Err(Error::Io(err)),
    };
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix("log.") else {
            continue;
        };
        let Some(num) = rest.strip_suffix(".jsonl.gz") else {
            continue;
        };
        if let Ok(n) = num.parse::<u32>() {
            rotated.push((n, entry.path()));
        }
    }
    rotated.sort_by_key(|(n, _)| *n);

    let mut inputs: Vec<PathBuf> = rotated.into_iter().map(|(_, p)| p).collect();
    let active = memory_dir.join("log.jsonl");
    if active.exists() {
        inputs.push(active);
    }
    let events = memory_dir.join("events.jsonl");
    if events.exists() {
        inputs.push(events);
    }
    let observations = memory_dir.join("observations.jsonl");
    if observations.exists() {
        inputs.push(observations);
    }
    let agents_dir = memory_dir.join("agents");
    if agents_dir.exists() {
        let mut agent_dirs: Vec<PathBuf> = std::fs::read_dir(&agents_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        agent_dirs.sort();
        for agent_dir in agent_dirs {
            let log = agent_dir.join("log.jsonl");
            if log.exists() {
                inputs.push(log);
            }
        }
    }
    Ok(inputs)
}

fn open_log_reader(path: &Path) -> Result<Box<dyn std::io::BufRead>, Error> {
    use std::io::BufReader;
    let file = std::fs::File::open(path)?;
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".gz"))
    {
        let decoder = flate2::read::GzDecoder::new(file);
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn apply_log_line(
    conn: &Connection,
    line: &str,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|err| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: err.to_string(),
        })?;
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: "missing 'kind'".to_string(),
        })?
        .to_string();

    match kind.as_str() {
        "add_claim" => apply_add_claim(conn, &value, path, lineno, report),
        "record_event" => apply_record_event(conn, &value, path, lineno, report),
        "record_observation" => apply_record_observation(conn, &value, path, lineno, report),
        other => Err(Error::ReplayUnknownKind {
            kind: other.to_string(),
        }),
    }
}

fn replay_field<'a>(
    value: &'a serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<&'a serde_json::Value, Error> {
    value.get(field).ok_or_else(|| Error::ReplayMalformed {
        path: path.to_string(),
        line: lineno,
        detail: format!("missing '{field}'"),
    })
}

fn replay_str<'a>(
    value: &'a serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<&'a str, Error> {
    replay_field(value, field, path, lineno)?
        .as_str()
        .ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("'{field}' is not a string"),
        })
}

fn replay_i64(
    value: &serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<i64, Error> {
    replay_field(value, field, path, lineno)?
        .as_i64()
        .ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("'{field}' is not an integer"),
        })
}

fn replay_f64(
    value: &serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<f64, Error> {
    replay_field(value, field, path, lineno)?
        .as_f64()
        .ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("'{field}' is not a number"),
        })
}

fn apply_add_claim(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let claim_id = replay_i64(value, "claim_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let subject = replay_str(value, "subject", path, lineno)?;
    let predicate = replay_str(value, "predicate", path, lineno)?;
    let object = replay_str(value, "object", path, lineno)?;
    let source = replay_str(value, "source", path, lineno)?;
    let agent_id = replay_str(value, "agent_id", path, lineno)?;
    let agent_kind_str = replay_str(value, "agent_kind", path, lineno)?;
    let confidence = replay_f64(value, "confidence", path, lineno)?;
    let agent_kind: AgentKind = agent_kind_str.parse()?;
    // Provenance is not in the log; derive from agent_kind. This is correct
    // for `insert_claim` writes; for promoted candidates it can diverge from
    // the original (candidate.provenance defaulted to INFERRED).
    let provenance = provenance_for_agent_kind(agent_kind);

    // Idempotency: if claim already exists, accept identical content,
    // otherwise fail loudly with E_REPLAY_DUPLICATE_ID.
    let existing: Option<(String, String, String, String, f64, String, String)> = conn
        .query_row(
            "SELECT subject, predicate, object, provenance, confidence, agent_id, agent_kind
               FROM claims WHERE id = ?1",
            [claim_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == subject
            && existing.1 == predicate
            && existing.2 == object
            && existing.3 == provenance.as_str()
            && (existing.4 - confidence).abs() < 1e-9
            && existing.5 == agent_id
            && existing.6 == agent_kind.as_str();
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("claim_id={claim_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }

    // Ensure entities exist (mirrors insert_claim's behavior). Use a tiny
    // inline ensure: insert if missing with the only-known type "Thing".
    ensure_entity_for_replay(conn, subject, ts)?;
    ensure_entity_for_replay(conn, object, ts)?;

    // ADR-0018: replay must rebuild the same `predicate_types` rows the
    // original write produced. USER_ASSERTED writes auto-extended the
    // ontology; replay applies the same policy so the trigger does not
    // fire on the subsequent INSERT. EXTRACTED/INFERRED rows in the log
    // were already accepted under the original ontology — replay accepts
    // them too (the log is the source of truth, ADR-0005).
    ontology_check_for_replay(conn, predicate, provenance, agent_id, ts)?;

    let source_refs = serde_json::to_string(&[source])?;
    conn.execute(
        "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                             status, source_refs, agent_id, agent_kind, write_ts,
                             created_at, last_seen_at, last_verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7,
                 ?8, ?9, ?10, ?10, ?10, ?10)",
        params![
            claim_id,
            subject,
            predicate,
            object,
            provenance.as_str(),
            confidence,
            source_refs,
            agent_id,
            agent_kind.as_str(),
            ts
        ],
    )?;
    report.claims += 1;
    Ok(())
}

/// Replay-side ontology check (ADR-0018). Mirrors `Store::ontology_check`
/// but works on a bare `Connection` because replay does not own a `Store`.
/// Replay must accept whatever the log says; for predicates absent from
/// `predicate_types` and `predicate_alias` it auto-extends regardless of
/// provenance, because the log records that the original write was
/// accepted at the time. This is more permissive than the live writer,
/// but it's the price of "log is source of truth" (ADR-0005).
fn ontology_check_for_replay(
    conn: &Connection,
    predicate: &str,
    _provenance: Provenance,
    agent_id: &str,
    ts: i64,
) -> Result<(), Error> {
    let known: Option<i64> = conn
        .query_row(
            "SELECT id FROM predicate_types WHERE name = ?1",
            [predicate],
            |row| row.get(0),
        )
        .optional()?;
    if known.is_some() {
        return Ok(());
    }
    let alias_hit: Option<i64> = conn
        .query_row(
            "SELECT predicate_id FROM predicate_alias WHERE alias = ?1",
            [predicate],
            |row| row.get(0),
        )
        .optional()?;
    if alias_hit.is_some() {
        return Ok(());
    }
    let parent_id: i64 = conn.query_row(
        "SELECT id FROM predicate_types WHERE name = 'relates_to'",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO predicate_types (name, parent_id, created_via, created_at)
         VALUES (?1, ?2, 'replay', ?3)",
        params![predicate, parent_id, ts],
    )?;
    seed::rebuild_closure(conn, "predicate_types", "predicate_closure")?;
    conn.execute(
        "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
         VALUES (?1, 'relates_to', ?2, ?3)",
        params![predicate, agent_id, ts],
    )?;
    Ok(())
}

fn ensure_entity_for_replay(conn: &Connection, name: &str, ts: i64) -> Result<(), Error> {
    // Look up the default "Thing" type; entities table requires type_id.
    let type_id: i64 = conn
        .query_row(
            "SELECT id FROM entity_types WHERE name = 'Thing'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(1);
    conn.execute(
        "INSERT OR IGNORE INTO entities (name, type_id, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?3)",
        params![name, type_id, ts],
    )?;
    Ok(())
}

fn apply_record_event(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let event_id = replay_i64(value, "event_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let session_id = replay_str(value, "session_id", path, lineno)?;
    let event_kind = replay_str(value, "event_kind", path, lineno)?;
    let payload = replay_str(value, "payload", path, lineno)?;
    let source = replay_str(value, "source", path, lineno)?;
    let agent_id = replay_str(value, "agent_id", path, lineno)?;
    let agent_kind = replay_str(value, "agent_kind", path, lineno)?;

    let existing: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT session_id, kind, payload, source, agent_id, agent_kind
               FROM episodic_events WHERE id = ?1",
            [event_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id
            && existing.1 == event_kind
            && existing.2 == payload
            && existing.3 == source
            && existing.4 == agent_id
            && existing.5 == agent_kind;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("event_id={event_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }
    conn.execute(
        "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event_id,
            session_id,
            event_kind,
            payload,
            source,
            agent_id,
            agent_kind,
            ts
        ],
    )?;
    report.events += 1;
    Ok(())
}

fn apply_record_observation(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let id = replay_str(value, "observation_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let session_id = replay_str(value, "session_id", path, lineno)?;
    let content = replay_str(value, "content", path, lineno)?;
    let relevance = replay_str(value, "relevance", path, lineno)?;
    let source_event_ids = replay_field(value, "source_event_ids", path, lineno)?;
    let source_event_ids_json = source_event_ids.to_string();
    let agent_id = replay_str(value, "agent_id", path, lineno)?;
    let agent_kind = replay_str(value, "agent_kind", path, lineno)?;
    let derivation = replay_str(value, "derivation", path, lineno)?;

    let existing: Option<(String, String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation
               FROM observations WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id
            && existing.1 == content
            && existing.2 == relevance
            && existing.3 == source_event_ids_json
            && existing.4 == agent_id
            && existing.5 == agent_kind
            && existing.6 == derivation;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("observation_id={id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }
    conn.execute(
        "INSERT INTO observations
         (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            session_id,
            content,
            relevance,
            source_event_ids_json,
            agent_id,
            agent_kind,
            derivation,
            ts
        ],
    )?;
    report.observations += 1;
    Ok(())
}
