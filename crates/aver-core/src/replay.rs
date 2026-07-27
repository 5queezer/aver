//! Log replay: parsers, apply handlers, strict/lenient modes.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::claims::provenance_for_agent_kind;
use crate::error::Error;
use crate::log::{AverLock, ConfidenceChange, ROTATED_LOG_SERIES};
use crate::ontology::ensure_entity_on;
use crate::seed;
use crate::seed::seed_ontology;
use crate::store::{EMBEDDED_MIGRATIONS as MIGRATIONS, ensure_sqlite_vec_registered};
use crate::types::{AgentKind, HyperedgeParticipantInput, Provenance};

/// Stats reported by `aver replay`. ADR-0019 §4.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    pub claims: u64,
    pub hyperedges: u64,
    pub events: u64,
    pub observations: u64,
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

pub(crate) fn collect_replay_inputs(memory_dir: &Path) -> Result<Vec<PathBuf>, Error> {
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

pub(crate) fn open_log_reader(path: &Path) -> Result<Box<dyn std::io::BufRead>, Error> {
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

pub(crate) fn apply_log_line(
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
        "add_hyperedge" => apply_add_hyperedge(conn, &value, path, lineno, report),
        "record_event" => apply_record_event(conn, &value, path, lineno, report),
        "record_observation" => apply_record_observation(conn, &value, path, lineno, report),
        "prune_observations" => apply_prune_observations(conn, &value, path, lineno, report),
        "retire_claim" => apply_retire_claim(conn, &value, path, lineno, report),
        "add_contradiction" => apply_add_contradiction(conn, &value, path, lineno, report),
        "propose_candidate_claim" => {
            apply_propose_candidate_claim(conn, &value, path, lineno, report)
        }
        "promote_candidate_claim" => {
            apply_promote_candidate_claim(conn, &value, path, lineno, report)
        }
        "reject_candidate_claim" => {
            apply_reject_candidate_claim(conn, &value, path, lineno, report)
        }
        "supersede_claims" => apply_supersede_claims(conn, &value, path, lineno, report),
        "decay_confidence" => apply_decay_confidence(conn, &value, path, lineno, report),
        "merge_source_refs" => apply_merge_source_refs(conn, &value, path, lineno, report),
        other => Err(Error::ReplayUnknownKind {
            kind: other.to_string(),
        }),
    }
}

pub(crate) fn replay_field<'a>(
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

pub(crate) fn replay_str<'a>(
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

pub(crate) fn replay_i64(
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

pub(crate) fn replay_f64(
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

/// ADR-0021: scope is optional in the log for backward compatibility —
/// lines written before the field existed replay as 'global', matching the
/// migration-0085 column default.
pub(crate) fn replay_scope<'a>(
    value: &'a serde_json::Value,
    path: &str,
    lineno: usize,
) -> Result<&'a str, Error> {
    match value.get("scope") {
        None => Ok("global"),
        Some(raw) => raw.as_str().ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: "'scope' is not a string".to_string(),
        }),
    }
}

pub(crate) fn replay_i64_list(
    value: &serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<Vec<i64>, Error> {
    serde_json::from_value(replay_field(value, field, path, lineno)?.clone()).map_err(|err| {
        Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("'{field}' is not an integer array: {err}"),
        }
    })
}

pub(crate) fn replay_string_list(
    value: &serde_json::Value,
    field: &str,
    path: &str,
    lineno: usize,
) -> Result<Vec<String>, Error> {
    serde_json::from_value(replay_field(value, field, path, lineno)?.clone()).map_err(|err| {
        Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("'{field}' is not a string array: {err}"),
        }
    })
}

pub(crate) fn apply_add_claim(
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
    // Provenance is explicit in current log lines; fall back to the
    // agent_kind derivation for lines written before the field existed.
    let provenance = match value.get("provenance") {
        None => provenance_for_agent_kind(agent_kind),
        Some(_) => {
            let raw = replay_str(value, "provenance", path, lineno)?;
            raw.parse::<Provenance>()
                .map_err(|_| Error::ReplayMalformed {
                    path: path.to_string(),
                    line: lineno,
                    detail: format!("unknown 'provenance' value: {raw}"),
                })?
        }
    };
    // Scope is explicit in current log lines; pre-scope lines replay as
    // 'global', matching the migration-0085 column default (ADR-0021).
    let scope = replay_scope(value, path, lineno)?;

    // Idempotency: if claim already exists, accept identical content,
    // otherwise fail loudly with E_REPLAY_DUPLICATE_ID. Only write-time
    // immutable fields are compared: lifecycle records (decay, merge,
    // retire) legitimately change provenance/confidence/source_refs/status
    // after the add_claim line, and per-agent log duplicates of the line
    // replay after those mutations.
    let existing: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT subject, predicate, object, agent_id, agent_kind, scope
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
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == subject
            && existing.1 == predicate
            && existing.2 == object
            && existing.3 == agent_id
            && existing.4 == agent_kind.as_str()
            && existing.5 == scope;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("claim_id={claim_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }

    // Ensure entities exist, mirroring insert_claim's behavior through the
    // shared ensure_entity implementation (type inference + requires_review).
    ensure_entity_on(conn, subject, ts)?;
    ensure_entity_on(conn, object, ts)?;

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
                             created_at, last_seen_at, last_verified_at, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7,
                 ?8, ?9, ?10, ?10, ?10, ?10, ?11)",
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
            ts,
            scope
        ],
    )?;
    report.claims += 1;
    Ok(())
}

pub(crate) fn apply_add_hyperedge(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let hyperedge_id = replay_i64(value, "hyperedge_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let predicate = replay_str(value, "predicate", path, lineno)?;
    let provenance_str = replay_str(value, "provenance", path, lineno)?;
    let provenance: Provenance = provenance_str.parse()?;
    let confidence = replay_f64(value, "confidence", path, lineno)?;
    let source_refs: Vec<String> = serde_json::from_value(
        replay_field(value, "source_refs", path, lineno)?.clone(),
    )
    .map_err(|err| Error::ReplayMalformed {
        path: path.to_string(),
        line: lineno,
        detail: format!("'source_refs' is not a string array: {err}"),
    })?;
    let participants: Vec<HyperedgeParticipantInput> =
        serde_json::from_value(replay_field(value, "participants", path, lineno)?.clone())
            .map_err(|err| Error::ReplayMalformed {
                path: path.to_string(),
                line: lineno,
                detail: format!("'participants' is not a participant array: {err}"),
            })?;

    let existing: Option<(String, String, f64, String)> = conn
        .query_row(
            "SELECT predicate, provenance, confidence, source_refs
               FROM hyperedges WHERE id = ?1",
            [hyperedge_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let existing_sources: Vec<String> = serde_json::from_str(&existing.3)?;
        let existing_participants = replay_hyperedge_participants(conn, hyperedge_id)?;
        let matches = existing.0 == predicate
            && existing.1 == provenance.as_str()
            && (existing.2 - confidence).abs() < 1e-9
            && existing_sources == source_refs
            && existing_participants == participants;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("hyperedge_id={hyperedge_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }

    ontology_check_for_replay(conn, predicate, provenance, "local", ts)?;
    let source_refs_json = serde_json::to_string(&source_refs)?;
    conn.execute(
        "INSERT INTO hyperedges (id, predicate, provenance, confidence, source_refs, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'ACTIVE', ?6, ?6)",
        params![
            hyperedge_id,
            predicate,
            provenance.as_str(),
            confidence,
            source_refs_json,
            ts,
        ],
    )?;
    for participant in &participants {
        ensure_entity_on(conn, &participant.entity, ts)?;
        conn.execute(
            "INSERT INTO hyperedge_participants (hyperedge_id, role, entity)
             VALUES (?1, ?2, ?3)",
            params![hyperedge_id, participant.role, participant.entity],
        )?;
    }
    report.hyperedges += 1;
    Ok(())
}

pub(crate) fn replay_hyperedge_participants(
    conn: &Connection,
    hyperedge_id: i64,
) -> Result<Vec<HyperedgeParticipantInput>, Error> {
    let mut stmt = conn.prepare(
        "SELECT role, entity
           FROM hyperedge_participants
          WHERE hyperedge_id = ?1
          ORDER BY id",
    )?;
    Ok(stmt
        .query_map([hyperedge_id], |row| {
            Ok(HyperedgeParticipantInput {
                role: row.get(0)?,
                entity: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Replay-side ontology check (ADR-0018). Mirrors `Store::ontology_check`
/// but works on a bare `Connection` because replay does not own a `Store`.
/// Replay must accept whatever the log says; for predicates absent from
/// `predicate_types` and `predicate_alias` it auto-extends regardless of
/// provenance, because the log records that the original write was
/// accepted at the time. This is more permissive than the live writer,
/// but it's the price of "log is source of truth" (ADR-0005).
pub(crate) fn ontology_check_for_replay(
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

pub(crate) fn apply_record_event(
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
    let scope = replay_scope(value, path, lineno)?;

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
        "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event_id,
            session_id,
            event_kind,
            payload,
            source,
            agent_id,
            agent_kind,
            ts,
            scope
        ],
    )?;
    report.events += 1;
    Ok(())
}

pub(crate) fn apply_record_observation(
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
    let scope = replay_scope(value, path, lineno)?;

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
         (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            session_id,
            content,
            relevance,
            source_event_ids_json,
            agent_id,
            agent_kind,
            derivation,
            ts,
            scope
        ],
    )?;
    report.observations += 1;
    Ok(())
}

pub(crate) fn apply_prune_observations(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    _report: &mut ReplayReport,
) -> Result<(), Error> {
    let marker_id = replay_str(value, "prune_marker_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let session_id = replay_str(value, "session_id", path, lineno)?;
    let pruned_observation_ids = replay_field(value, "pruned_observation_ids", path, lineno)?;
    let pruned_observation_ids_json = pruned_observation_ids.to_string();

    let existing: Option<(String, String, String)> = conn
        .query_row(
            "SELECT session_id, pruned_observation_ids, ts
               FROM observation_prune_markers WHERE id = ?1",
            [marker_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)?.to_string())),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id
            && existing.1 == pruned_observation_ids_json
            && existing.2 == ts.to_string();
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("prune_marker_id={marker_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }
    conn.execute(
        "INSERT INTO observation_prune_markers
         (id, session_id, pruned_observation_ids, ts)
         VALUES (?1, ?2, ?3, ?4)",
        params![marker_id, session_id, pruned_observation_ids_json, ts],
    )?;
    Ok(())
}

/// Replay a `retire_claim` record (ADR-0023): reproduce the live path's
/// source_refs marker append + INVALIDATED flip against the already-replayed
/// claim state.
pub(crate) fn apply_retire_claim(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let claim_id = replay_i64(value, "claim_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let reason = replay_str(value, "reason", path, lineno)?;

    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT status, source_refs FROM claims WHERE id = ?1",
            [claim_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, source_refs_json)) = existing else {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("retire_claim references missing claim {claim_id}"),
        });
    };
    let marker = format!("retired:{reason}");
    if status == "INVALIDATED" {
        // Idempotency: the same retirement is only valid once.
        let refs: Vec<String> = serde_json::from_str(&source_refs_json)?;
        if refs.iter().any(|source_ref| source_ref == &marker) {
            return Ok(());
        }
        return Err(Error::ReplayDuplicateId {
            detail: format!("claim_id={claim_id} already INVALIDATED at {path}:{lineno}"),
        });
    }
    let mut refs: Vec<String> = serde_json::from_str(&source_refs_json)?;
    refs.push(marker);
    conn.execute(
        "UPDATE claims
            SET status = 'INVALIDATED',
                source_refs = ?1,
                last_seen_at = ?2
          WHERE id = ?3",
        params![serde_json::to_string(&refs)?, ts, claim_id],
    )?;
    report.lifecycle += 1;
    Ok(())
}

pub(crate) fn apply_add_contradiction(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let contradiction_id = replay_i64(value, "contradiction_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let claim_id = replay_i64(value, "claim_id", path, lineno)?;
    let reason = replay_str(value, "reason", path, lineno)?;
    let new_claim_id: Option<i64> = match value.get("new_claim_id") {
        None => {
            return Err(Error::ReplayMalformed {
                path: path.to_string(),
                line: lineno,
                detail: "missing 'new_claim_id'".to_string(),
            });
        }
        Some(raw) if raw.is_null() => None,
        Some(raw) => Some(raw.as_i64().ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: "'new_claim_id' is not an integer".to_string(),
        })?),
    };

    let existing: Option<(i64, String, Option<i64>)> = conn
        .query_row(
            "SELECT claim_id, reason, new_claim_id FROM contradictions WHERE id = ?1",
            [contradiction_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == claim_id && existing.1 == reason && existing.2 == new_claim_id;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!(
                    "contradiction_id={contradiction_id} content mismatch at {path}:{lineno}"
                ),
            });
        }
        return Ok(());
    }
    conn.execute(
        "INSERT INTO contradictions (id, claim_id, reason, new_claim_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![contradiction_id, claim_id, reason, new_claim_id, ts],
    )?;
    report.lifecycle += 1;
    Ok(())
}

pub(crate) fn apply_propose_candidate_claim(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let candidate_id = replay_i64(value, "candidate_id", path, lineno)?;
    let ts = replay_i64(value, "ts", path, lineno)?;
    let event_id = replay_i64(value, "event_id", path, lineno)?;
    let subject = replay_str(value, "subject", path, lineno)?;
    let predicate = replay_str(value, "predicate", path, lineno)?;
    let object = replay_str(value, "object", path, lineno)?;
    let scope = replay_scope(value, path, lineno)?;

    let existing: Option<(i64, String, String, String, String)> = conn
        .query_row(
            "SELECT event_id, subject, predicate, object, scope
               FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == event_id
            && existing.1 == subject
            && existing.2 == predicate
            && existing.3 == object
            && existing.4 == scope;
        if !matches {
            return Err(Error::ReplayDuplicateId {
                detail: format!("candidate_id={candidate_id} content mismatch at {path}:{lineno}"),
            });
        }
        return Ok(());
    }
    // provenance/confidence/status stay at the schema defaults (INFERRED,
    // 0.45, PENDING) — the live proposal path never overrides them.
    conn.execute(
        "INSERT INTO candidate_claims (id, event_id, subject, predicate, object, created_at, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            candidate_id,
            event_id,
            subject,
            predicate,
            object,
            ts,
            scope
        ],
    )?;
    report.lifecycle += 1;
    Ok(())
}

pub(crate) fn apply_promote_candidate_claim(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let candidate_id = replay_i64(value, "candidate_id", path, lineno)?;
    let _ts = replay_i64(value, "ts", path, lineno)?;
    let claim_id = replay_i64(value, "claim_id", path, lineno)?;

    let existing: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT status, promoted_claim_id FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, promoted_claim_id)) = existing else {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("promote_candidate_claim references missing candidate {candidate_id}"),
        });
    };
    if status == "PROMOTED" {
        if promoted_claim_id == Some(claim_id) {
            return Ok(());
        }
        return Err(Error::ReplayDuplicateId {
            detail: format!(
                "candidate_id={candidate_id} already PROMOTED to a different claim at {path}:{lineno}"
            ),
        });
    }
    if status != "PENDING" {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("cannot promote candidate {candidate_id} from status {status}"),
        });
    }
    let claim_exists = conn
        .query_row("SELECT 1 FROM claims WHERE id = ?1", [claim_id], |_| Ok(()))
        .optional()?
        .is_some();
    if !claim_exists {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("promotion references missing claim {claim_id}"),
        });
    }
    let rows_changed = conn.execute(
        "UPDATE candidate_claims
            SET status = 'PROMOTED', promoted_claim_id = ?1
          WHERE id = ?2 AND status = 'PENDING'",
        params![claim_id, candidate_id],
    )?;
    if rows_changed != 1 {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("promotion changed {rows_changed} rows for candidate {candidate_id}"),
        });
    }
    report.lifecycle += 1;
    Ok(())
}

pub(crate) fn apply_reject_candidate_claim(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let candidate_id = replay_i64(value, "candidate_id", path, lineno)?;
    let _ts = replay_i64(value, "ts", path, lineno)?;
    let reason = replay_str(value, "reason", path, lineno)?;

    let existing: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, rejection_reason FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, rejection_reason)) = existing else {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("reject_candidate_claim references missing candidate {candidate_id}"),
        });
    };
    if status == "REJECTED" {
        if rejection_reason.as_deref() == Some(reason) {
            return Ok(());
        }
        return Err(Error::ReplayDuplicateId {
            detail: format!(
                "candidate_id={candidate_id} already REJECTED with a different reason at {path}:{lineno}"
            ),
        });
    }
    if status != "PENDING" {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("cannot reject candidate {candidate_id} from status {status}"),
        });
    }
    let rows_changed = conn.execute(
        "UPDATE candidate_claims
            SET status = 'REJECTED', rejection_reason = ?1
          WHERE id = ?2 AND status = 'PENDING'",
        params![reason, candidate_id],
    )?;
    if rows_changed != 1 {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("rejection changed {rows_changed} rows for candidate {candidate_id}"),
        });
    }
    report.lifecycle += 1;
    Ok(())
}

/// Replay a `supersede_claims` record: assert the logged lifecycle
/// transitions. Naturally idempotent (the UPDATE is a no-op once the claim
/// is SUPERSEDED) and tolerant of claims quarantined in lenient mode.
pub(crate) fn apply_supersede_claims(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let _ts = replay_i64(value, "ts", path, lineno)?;
    let claim_ids = replay_i64_list(value, "claim_ids", path, lineno)?;
    for claim_id in &claim_ids {
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM claims WHERE id = ?1",
                [claim_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Err(Error::ReplayMalformed {
                path: path.to_string(),
                line: lineno,
                detail: format!("supersede_claims references missing claim {claim_id}"),
            });
        };
        if status == "SUPERSEDED" {
            continue;
        }
        let rows_changed = conn.execute(
            "UPDATE claims SET status = 'SUPERSEDED' WHERE id = ?1",
            [claim_id],
        )?;
        if rows_changed != 1 {
            return Err(Error::ReplayMalformed {
                path: path.to_string(),
                line: lineno,
                detail: format!("supersede changed {rows_changed} rows for claim {claim_id}"),
            });
        }
    }
    report.lifecycle += 1;
    Ok(())
}

/// Replay a `decay_confidence` record: apply the logged post-decay values
/// verbatim — the log records outcomes, not formulas.
pub(crate) fn apply_decay_confidence(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let _ts = replay_i64(value, "ts", path, lineno)?;
    let changes: Vec<ConfidenceChange> = serde_json::from_value(
        replay_field(value, "changes", path, lineno)?.clone(),
    )
    .map_err(|err| Error::ReplayMalformed {
        path: path.to_string(),
        line: lineno,
        detail: format!("'changes' is not a confidence-change array: {err}"),
    })?;
    for change in &changes {
        let rows_changed = conn.execute(
            "UPDATE claims SET confidence = ?1 WHERE id = ?2",
            params![change.confidence, change.claim_id],
        )?;
        if rows_changed != 1 {
            return Err(Error::ReplayMalformed {
                path: path.to_string(),
                line: lineno,
                detail: format!(
                    "decay_confidence references missing claim {}",
                    change.claim_id
                ),
            });
        }
    }
    report.lifecycle += 1;
    Ok(())
}

pub(crate) fn apply_merge_source_refs(
    conn: &Connection,
    value: &serde_json::Value,
    path: &str,
    lineno: usize,
    report: &mut ReplayReport,
) -> Result<(), Error> {
    let _ts = replay_i64(value, "ts", path, lineno)?;
    let claim_id = replay_i64(value, "claim_id", path, lineno)?;
    let source_refs = replay_string_list(value, "source_refs", path, lineno)?;
    let promote = replay_field(value, "promote", path, lineno)?
        .as_bool()
        .ok_or_else(|| Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: "'promote' is not a boolean".to_string(),
        })?;
    let merged = serde_json::to_string(&source_refs)?;
    let rows_changed = if promote {
        conn.execute(
            "UPDATE claims
                SET source_refs = ?1, provenance = 'EXTRACTED', confidence = MAX(confidence, 0.75)
              WHERE id = ?2",
            params![merged, claim_id],
        )?
    } else {
        conn.execute(
            "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
            params![merged, claim_id],
        )?
    };
    if rows_changed != 1 {
        return Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!("merge_source_refs references missing claim {claim_id}"),
        });
    }
    report.lifecycle += 1;
    Ok(())
}
