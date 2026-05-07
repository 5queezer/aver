//! Memory layer core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

/// Embedded migrations applied in order on every `Store::open`.
/// Each `CREATE` is `IF NOT EXISTS` so re-application is a no-op (ADR-0005).
const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_init",
    include_str!("../../../migrations/0001_init.sql"),
)];

/// Local storage for the memory layer (ADR-0006).
///
/// Layout under `memory_dir`:
///   db.sqlite  — claims, entities, episodes, contradictions
///   log.jsonl  — append-only audit log (ADR-0005, source of truth)
pub struct Store {
    conn: Connection,
    log_path: PathBuf,
}

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
            Self::Extracted    => "EXTRACTED",
            Self::Inferred     => "INFERRED",
            Self::Ambiguous    => "AMBIGUOUS",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "USER_ASSERTED" => Ok(Self::UserAsserted),
            "EXTRACTED"     => Ok(Self::Extracted),
            "INFERRED"      => Ok(Self::Inferred),
            "AMBIGUOUS"     => Ok(Self::Ambiguous),
            other           => Err(Error::EnumParse {
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
            Self::Active      => "ACTIVE",
            Self::Superseded  => "SUPERSEDED",
            Self::Invalidated => "INVALIDATED",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "ACTIVE"      => Ok(Self::Active),
            "SUPERSEDED"  => Ok(Self::Superseded),
            "INVALIDATED" => Ok(Self::Invalidated),
            other         => Err(Error::EnumParse {
                kind: "ClaimStatus",
                value: other.to_string(),
            }),
        }
    }
}

impl Store {
    /// Open or create a memory store rooted at `memory_dir`.
    /// The directory is created if it does not exist; migrations are applied.
    pub fn open(memory_dir: impl AsRef<Path>) -> Result<Self, Error> {
        let memory_dir = memory_dir.as_ref();
        std::fs::create_dir_all(memory_dir)?;

        let db_path = memory_dir.join("db.sqlite");
        let log_path = memory_dir.join("log.jsonl");

        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        for (_name, sql) in MIGRATIONS {
            conn.execute_batch(sql)?;
        }

        Ok(Self { conn, log_path })
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
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Pre-allocate the claim id. Single-writer assumption: rusqlite's
        // Connection is !Sync, so within a process this is race-free; SQLite
        // WAL serializes writers across processes.
        let claim_id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM claims",
            [],
            |r| r.get(0),
        )?;

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            claim_id,
            subject,
            predicate,
            object,
            source,
        };
        append_jsonl(&self.log_path, &entry)?;

        let source_refs = serde_json::to_string(&[source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 'USER_ASSERTED', 0.95, 'ACTIVE', ?5, ?6, ?6)",
            params![claim_id, subject, predicate, object, source_refs, now],
        )?;
        Ok(claim_id)
    }

    /// Retrieve a claim by id.
    pub fn get_claim(&self, id: i64) -> Result<Claim, Error> {
        let (id, subject, predicate, object, provenance, confidence, status, source_refs): (
            i64, String, String, String, String, f64, String, String,
        ) = self.conn.query_row(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs
               FROM claims WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                    row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                ))
            },
        )?;

        Ok(Claim {
            id,
            subject,
            predicate,
            object,
            provenance: Provenance::from_str(&provenance)?,
            confidence,
            status: ClaimStatus::from_str(&status)?,
            source_refs: serde_json::from_str(&source_refs)?,
        })
    }
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
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
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
    #[error("invalid {kind} value in database: {value:?}")]
    EnumParse { kind: &'static str, value: String },
}
