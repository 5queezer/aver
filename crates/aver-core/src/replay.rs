//! Log replay (ADR-0019 §4): rebuild the SQLite projection from JSONL logs.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::apply::{Applied, apply};
use crate::error::Error;
use crate::log::{AverLock, LogRecord, ROTATED_LOG_SERIES};
use crate::seed::seed_ontology;
use crate::store::{SCHEMA_VERSION, ensure_sqlite_vec_registered, pending_migrations};
/// Stats reported by `aver replay`. ADR-0019 §4.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    pub claims: u64,
    pub hyperedges: u64,
    pub events: u64,
    pub observations: u64,
    /// Vector chunk records applied (`add_vector_chunk` + `embed_vector_chunk`).
    pub vector_chunks: u64,
    pub files_walked: u64,
    /// Lifecycle records applied (retire/contradiction/candidate/supersede/
    /// decay/merge transitions).
    pub lifecycle: u64,
    /// Lines quarantined in lenient mode. Always empty in strict mode: the
    /// first bad line aborts the run instead.
    pub quarantined: Vec<ReplayQuarantine>,
}

/// A log line that failed to apply in lenient replay mode, kept with enough
/// context to locate and repair it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayQuarantine {
    pub path: String,
    pub line: usize,
    pub error: String,
}

/// How `replay` handles a log line that fails to apply. ADR-0019 §4 pins
/// strict mode as the default; lenient mode exists for disaster recovery,
/// where one poisoned line must not block rebuilding every other projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplayMode {
    /// Abort on the first invalid line (default, historical behavior).
    #[default]
    Strict,
    /// Collect invalid lines into [`ReplayReport::quarantined`] and continue.
    Lenient,
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
///
/// Strict mode: the first line that fails to apply aborts the run. Use
/// [`replay_with_mode`] with [`ReplayMode::Lenient`] to quarantine bad lines
/// instead (disaster recovery).
pub fn replay(memory_dir: &Path, force: bool) -> Result<ReplayReport, Error> {
    replay_with_mode(memory_dir, force, ReplayMode::Strict)
}

/// `replay` with an explicit strictness mode. In lenient mode, lines that
/// fail to apply are collected into [`ReplayReport::quarantined`] with
/// path/line/error diagnostics and replay continues with the next line.
pub fn replay_with_mode(
    memory_dir: &Path,
    force: bool,
    mode: ReplayMode,
) -> Result<ReplayReport, Error> {
    use std::io::BufRead;

    // ADR-0017: replay creates a fresh DB and re-runs migrations, including
    // the 0010 vec0 virtual table. The extension must be registered before
    // the partial Connection is opened.
    ensure_sqlite_vec_registered();

    std::fs::create_dir_all(memory_dir)?;
    // Hold the advisory lock for the whole run (same as vacuum/rotation):
    // replay swaps in a fresh db.sqlite, so a live Store writing meanwhile
    // would race the swap.
    let _lock = AverLock::acquire(memory_dir)?;
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
        for sql in pending_migrations(0)? {
            conn.execute_batch(sql)?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
                conn.execute_batch("SAVEPOINT replay_line")?;
                let report_before = report.clone();
                let result = apply_log_line(
                    &conn,
                    &line,
                    &input.to_string_lossy(),
                    lineno + 1,
                    &mut report,
                );
                match result {
                    Ok(()) => conn.execute_batch("RELEASE replay_line")?,
                    Err(err) => {
                        conn.execute_batch("ROLLBACK TO replay_line; RELEASE replay_line")?;
                        report = report_before;
                        match mode {
                            ReplayMode::Strict => return Err(err),
                            ReplayMode::Lenient => report.quarantined.push(ReplayQuarantine {
                                path: input.to_string_lossy().into_owned(),
                                line: lineno + 1,
                                error: err.to_string(),
                            }),
                        }
                    }
                }
            }
        }

        conn.execute_batch("PRAGMA optimize")?;
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(report)
    })();

    match result {
        Ok(report) => {
            // Atomic swap: only overwrite db.sqlite after success.
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
    let mut inputs: Vec<PathBuf> = Vec::new();
    // Each series replays oldest-first: rotated archives in numeric order,
    // then the active file. Series order is fixed (claims before events
    // before observations) so records apply after the rows they reference.
    for series in ROTATED_LOG_SERIES {
        let mut rotated: Vec<(u32, PathBuf)> = Vec::new();
        let read_dir = match std::fs::read_dir(memory_dir) {
            Ok(rd) => rd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(err) => return Err(Error::Io(err)),
        };
        let prefix = format!("{series}.");
        for entry in read_dir {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name.strip_prefix(&prefix) else {
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
        inputs.extend(rotated.into_iter().map(|(_, p)| p));
        let active = memory_dir.join(format!("{series}.jsonl"));
        if active.exists() {
            inputs.push(active);
        }
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

/// Parse one log line. Unknown `kind` tags and malformed lines are told
/// apart so operators see `E_REPLAY_UNKNOWN_KIND` for the former.
fn parse_log_line(line: &str, path: &str, lineno: usize) -> Result<LogRecord, Error> {
    let malformed = |detail: String| Error::ReplayMalformed {
        path: path.to_string(),
        line: lineno,
        detail,
    };
    match serde_json::from_str::<LogRecord>(line) {
        Ok(record) => Ok(record),
        Err(err) => {
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|err| malformed(err.to_string()))?;
            match value.get("kind").and_then(|v| v.as_str()) {
                None => Err(malformed("missing 'kind'".to_string())),
                Some(kind) if !LogRecord::KINDS.contains(&kind) => Err(Error::ReplayUnknownKind {
                    kind: kind.to_string(),
                }),
                Some(_) => Err(malformed(err.to_string())),
            }
        }
    }
}

/// Parse and apply one line through the single projection writer
/// (`apply::apply`), then count it. Identical duplicates are skipped and not
/// counted; apply's record-level errors are located to `path:line`.
fn apply_log_line(
    conn: &Connection,
    line: &str,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let record = parse_log_line(line, path, lineno)?;
    let applied = apply(conn, &record).map_err(|err| match err {
        Error::InvalidRecord { detail } => Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail,
        },
        Error::ReplayDuplicateId { detail } => Error::ReplayDuplicateId {
            detail: format!("{detail} at {path}:{lineno}"),
        },
        other => other,
    })?;
    if applied == Applied::Duplicate {
        return Ok(());
    }
    match record {
        LogRecord::AddClaim { .. } => report.claims += 1,
        LogRecord::AddHyperedge { .. } => report.hyperedges += 1,
        LogRecord::RecordEvent { .. } => report.events += 1,
        LogRecord::RecordObservation { .. } => report.observations += 1,
        LogRecord::AddVectorChunk { .. } | LogRecord::EmbedVectorChunk { .. } => {
            report.vector_chunks += 1
        }
        // Prune markers were never counted; keep the report contract.
        LogRecord::PruneObservations { .. } => {}
        LogRecord::RetireClaim { .. }
        | LogRecord::AddContradiction { .. }
        | LogRecord::ProposeCandidateClaim { .. }
        | LogRecord::PromoteCandidateClaim { .. }
        | LogRecord::RejectCandidateClaim { .. }
        | LogRecord::SupersedeClaims { .. }
        | LogRecord::DecayConfidence { .. }
        | LogRecord::MergeSourceRefs { .. } => report.lifecycle += 1,
    }
    Ok(())
}
