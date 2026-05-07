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
/// More fields will surface as later tests demand them.
#[derive(Debug, Clone)]
pub struct Claim {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
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

    /// Append a USER_ASSERTED claim. Writes the JSONL log line first
    /// (source of truth, ADR-0005), then mirrors into SQLite.
    /// Default confidence is 0.95 per ADR-0003's policy table.
    pub fn add_claim(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            subject,
            predicate,
            object,
            source,
        };
        append_jsonl(&self.log_path, &entry)?;

        let source_refs = serde_json::to_string(&[source])?;
        self.conn.execute(
            "INSERT INTO claims (subject, predicate, object, provenance, confidence,
                                 status, source_refs, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'USER_ASSERTED', 0.95, 'ACTIVE', ?4, ?5, ?5)",
            params![subject, predicate, object, source_refs, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieve a claim by id.
    pub fn get_claim(&self, id: i64) -> Result<Claim, Error> {
        self.conn
            .query_row(
                "SELECT id, subject, predicate, object FROM claims WHERE id = ?1",
                [id],
                |row| {
                    Ok(Claim {
                        id: row.get(0)?,
                        subject: row.get(1)?,
                        predicate: row.get(2)?,
                        object: row.get(3)?,
                    })
                },
            )
            .map_err(Error::from)
    }
}

#[derive(Serialize)]
struct LogEntry<'a> {
    kind: &'a str,
    ts: i64,
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
}
