//! Append-only JSONL audit log: record shapes, append, rotation, advisory lock.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate_decision;
use crate::error::Error;
use crate::types::HyperedgeParticipantInput;
/// Log rotation thresholds (ADR-0019 §5).
pub const LOG_ROTATE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const LOG_ROTATE_MAX_LINES: u64 = 500_000;

/// JSONL log series rotated at session boundaries (ADR-0019 §5): the claims
/// log plus the episodic event and observation logs, which otherwise grow
/// without bound. Order matters for replay: claims apply before events and
/// observations (candidate and observation records reference them).
pub(crate) const ROTATED_LOG_SERIES: [&str; 3] = ["log", "events", "observations"];

fn default_scope() -> String {
    "global".to_string()
}

/// One (claim_id, confidence) pair inside a `decay_confidence` record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConfidenceChange {
    pub(crate) claim_id: i64,
    pub(crate) confidence: f64,
}

/// ADR-0024 `decision` field on promote/reject candidate log lines:
/// `{"model": "...", "choice": "promote" | "reject"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Decision {
    pub(crate) model: String,
    pub(crate) choice: String,
}

impl From<candidate_decision::DecisionLogField<'_>> for Decision {
    fn from(field: candidate_decision::DecisionLogField<'_>) -> Self {
        Self {
            model: field.model.to_string(),
            choice: field.choice.to_string(),
        }
    }
}

/// Every record the append-only log can carry (ADR-0005, ADR-0026). The
/// JSON shape of each variant is frozen: `kind` is the serde tag, field
/// names and order are the on-disk format, and every log written so far
/// still parses. Fields added after a kind first shipped carry a serde
/// default that reproduces the behaviour the old lines had.
///
/// The record is the unit of truth: `apply::apply` turns one record into
/// projection rows, for live writes and replay alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LogRecord {
    AddClaim {
        ts: i64,
        claim_id: i64,
        subject: String,
        predicate: String,
        object: String,
        source: String,
        agent_id: String,
        agent_kind: String,
        confidence: f64,
        /// Explicit provenance: deriving it from `agent_kind` misclassifies
        /// promoted candidates (an INFERRED candidate sourced by a HUMAN
        /// event would replay as USER_ASSERTED). Lines written before the
        /// field existed carry `None` and fall back to that derivation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<String>,
        /// ADR-0021: lines written before scopes existed replay as "global".
        #[serde(default = "default_scope")]
        scope: String,
    },
    AddHyperedge {
        ts: i64,
        hyperedge_id: i64,
        predicate: String,
        provenance: String,
        confidence: f64,
        source_refs: Vec<String>,
        participants: Vec<HyperedgeParticipantInput>,
    },
    RecordEvent {
        ts: i64,
        event_id: i64,
        session_id: String,
        event_kind: String,
        payload: String,
        source: String,
        agent_id: String,
        agent_kind: String,
        #[serde(default = "default_scope")]
        scope: String,
    },
    RecordObservation {
        ts: i64,
        /// Content hash, not a minted id (see `episodic::observation_id`).
        observation_id: String,
        session_id: String,
        content: String,
        relevance: String,
        source_event_ids: Vec<i64>,
        agent_id: String,
        agent_kind: String,
        derivation: String,
        #[serde(default = "default_scope")]
        scope: String,
    },
    PruneObservations {
        ts: i64,
        prune_marker_id: String,
        session_id: String,
        pruned_observation_ids: Vec<String>,
    },
    /// ADR-0023 retirement. Apply re-derives the `retired:<reason>`
    /// source_refs marker from the claim state, so only the reason is logged.
    RetireClaim {
        ts: i64,
        claim_id: i64,
        reason: String,
    },
    /// ADR-0003/0005 contradiction audit record.
    AddContradiction {
        ts: i64,
        contradiction_id: i64,
        claim_id: i64,
        reason: String,
        new_claim_id: Option<i64>,
    },
    /// ADR-0005 staged candidate. Provenance and confidence are not logged:
    /// proposal always uses the schema defaults (INFERRED, 0.45).
    ProposeCandidateClaim {
        ts: i64,
        candidate_id: i64,
        event_id: i64,
        subject: String,
        predicate: String,
        object: String,
        #[serde(default = "default_scope")]
        scope: String,
    },
    /// Candidate status flip. The promoted claim itself is the `add_claim`
    /// record appended immediately before this one.
    PromoteCandidateClaim {
        ts: i64,
        candidate_id: i64,
        claim_id: i64,
        /// ADR-0024: absent for manual promotion.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<Decision>,
    },
    RejectCandidateClaim {
        ts: i64,
        candidate_id: i64,
        reason: String,
        /// ADR-0024: absent for manual rejection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<Decision>,
    },
    /// ADR-0005 consolidation supersede outcome as an explicit id list, so
    /// replay asserts the same transition instead of recomputing it.
    SupersedeClaims { ts: i64, claim_ids: Vec<i64> },
    /// ADR-0005 confidence-decay outcome: concrete post-decay values, not the
    /// formula, so the log stays the truth if the policy changes.
    DecayConfidence {
        ts: i64,
        changes: Vec<ConfidenceChange>,
    },
    /// ADR-0005 source_refs merge for one duplicate group. `promote` mirrors
    /// the live merge's INFERRED->EXTRACTED promotion branch.
    MergeSourceRefs {
        ts: i64,
        claim_id: i64,
        source_refs: Vec<String>,
        promote: bool,
    },
    /// ADR-0017/0026 vector chunk. Logged so replay rebuilds `vector_chunks`
    /// and the `vec0` index instead of leaving them empty.
    AddVectorChunk {
        ts: i64,
        chunk_id: i64,
        claim_id: i64,
        text: String,
        embedding_model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embedding: Option<Vec<f32>>,
    },
    /// Backfilled embedding for a chunk logged without one.
    EmbedVectorChunk {
        ts: i64,
        chunk_id: i64,
        embedding: Vec<f32>,
    },
}

impl LogRecord {
    /// Every `kind` tag, for distinguishing an unknown kind from a malformed
    /// line. Kept in sync with the enum by `record_tests::kinds_are_complete`.
    pub(crate) const KINDS: [&'static str; 15] = [
        "add_claim",
        "add_hyperedge",
        "record_event",
        "record_observation",
        "prune_observations",
        "retire_claim",
        "add_contradiction",
        "propose_candidate_claim",
        "promote_candidate_claim",
        "reject_candidate_claim",
        "supersede_claims",
        "decay_confidence",
        "merge_source_refs",
        "add_vector_chunk",
        "embed_vector_chunk",
    ];

    #[cfg(test)]
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::AddClaim { .. } => "add_claim",
            Self::AddHyperedge { .. } => "add_hyperedge",
            Self::RecordEvent { .. } => "record_event",
            Self::RecordObservation { .. } => "record_observation",
            Self::PruneObservations { .. } => "prune_observations",
            Self::RetireClaim { .. } => "retire_claim",
            Self::AddContradiction { .. } => "add_contradiction",
            Self::ProposeCandidateClaim { .. } => "propose_candidate_claim",
            Self::PromoteCandidateClaim { .. } => "promote_candidate_claim",
            Self::RejectCandidateClaim { .. } => "reject_candidate_claim",
            Self::SupersedeClaims { .. } => "supersede_claims",
            Self::DecayConfidence { .. } => "decay_confidence",
            Self::MergeSourceRefs { .. } => "merge_source_refs",
            Self::AddVectorChunk { .. } => "add_vector_chunk",
            Self::EmbedVectorChunk { .. } => "embed_vector_chunk",
        }
    }

    /// The newest writer-minted id this record carries (ADR-0026), used to
    /// seed the generator from a log tail. Records that only reference
    /// older ids return `None`.
    pub(crate) fn newest_minted_id(&self) -> Option<i64> {
        match self {
            Self::AddClaim { claim_id, .. } => Some(*claim_id),
            Self::AddHyperedge { hyperedge_id, .. } => Some(*hyperedge_id),
            Self::RecordEvent { event_id, .. } => Some(*event_id),
            Self::ProposeCandidateClaim { candidate_id, .. } => Some(*candidate_id),
            Self::PromoteCandidateClaim {
                candidate_id,
                claim_id,
                ..
            } => Some((*candidate_id).max(*claim_id)),
            Self::AddContradiction {
                contradiction_id, ..
            } => Some(*contradiction_id),
            Self::AddVectorChunk { chunk_id, .. } => Some(*chunk_id),
            Self::RecordObservation { .. }
            | Self::PruneObservations { .. }
            | Self::RetireClaim { .. }
            | Self::RejectCandidateClaim { .. }
            | Self::SupersedeClaims { .. }
            | Self::DecayConfidence { .. }
            | Self::MergeSourceRefs { .. }
            | Self::EmbedVectorChunk { .. } => None,
        }
    }
}

pub(crate) fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let parent = path.parent().unwrap_or(Path::new(""));
    std::fs::create_dir_all(parent)?;
    // Hold a shared lock across open → write → sync so rotation (and
    // replay/vacuum), which lock `.lock` exclusively, can never rename the
    // active log out from under an in-flight append.
    let _lock = lock_shared(parent)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

/// Block until a shared advisory lock on `<dir>/.lock` (the file
/// [`AverLock`] locks exclusively) is held; released when the handle drops.
/// Never truncates or deletes the file.
fn lock_shared(dir: &Path) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    file.lock_shared()?;
    Ok(file)
}

/// Last non-empty line of a file, read from the tail without loading the
/// whole file. `Ok(None)` for a missing or empty file.
pub(crate) fn last_line(path: &Path) -> std::io::Result<Option<String>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut end = file.metadata()?.len();
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    while end > 0 {
        let start = end.saturating_sub(buf.len() as u64);
        let len = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf[..len])?;
        tail.splice(0..0, buf[..len].iter().copied());
        end = start;
        // Stop once a newline precedes some non-whitespace content: the
        // last line is complete.
        let trimmed_len = tail
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(0, |i| i + 1);
        if tail[..trimmed_len].contains(&b'\n') {
            break;
        }
    }
    let text = String::from_utf8_lossy(&tail);
    Ok(text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::to_owned))
}

/// Largest writer-minted id in the last line of each active log series
/// (ADR-0026). Seeds the id generator so ids stay monotonic across process
/// restarts even if the clock moved backwards; the projection is never
/// consulted. Unparseable tails are ignored (the clock still orders ids).
pub(crate) fn max_id_in_log_tails(memory_dir: &Path) -> Result<i64, Error> {
    let mut max = 0;
    for series in ROTATED_LOG_SERIES {
        let Some(line) = last_line(&memory_dir.join(format!("{series}.jsonl")))? else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<LogRecord>(&line) else {
            continue;
        };
        if let Some(id) = record.newest_minted_id() {
            max = max.max(id);
        }
    }
    Ok(max)
}

/// Acquire the advisory `<memory_dir>/.lock` file (ADR-0019 §2/§5).
/// The OS lock is held by the retained file handle for the guard's lifetime;
/// the PID written into the file is informational only.
pub struct AverLock {
    // Dropping the handle releases the advisory lock. The marker path is never
    // deleted, so every contender locks the same inode.
    _file: File,
}

impl AverLock {
    /// Acquire `<memory_dir>/.lock` without relying on PID-file liveness races.
    /// Refuses with [`Error::LockHeld`] if another handle holds the OS lock.
    pub fn acquire(memory_dir: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(memory_dir)?;
        let path = memory_dir.join(".lock");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(Error::LockHeld {
                    path: path.display().to_string(),
                });
            }
            Err(std::fs::TryLockError::Error(err)) => return Err(Error::Io(err)),
        }
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { _file: file })
    }
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

/// Determine the next rotation index `N` for `{series}.{N}.jsonl[.gz]`.
fn next_rotation_index(memory_dir: &Path, series: &str) -> std::io::Result<u32> {
    let mut max = 0u32;
    let read_dir = match std::fs::read_dir(memory_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(1),
        Err(err) => return Err(err),
    };
    let prefix = format!("{series}.");
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Match {series}.{N}.jsonl or {series}.{N}.jsonl.gz
        let rest = match name.strip_prefix(&prefix) {
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

/// Recover from a partial rotation in any log series: a `{series}.{N}.jsonl`
/// without a matching `.gz` finishes gzipping; a `.gz` left truncated by a
/// crash mid-compression is rebuilt from its complete plain source instead
/// of silently winning over it. Stale `.tmp` archives from a crash between
/// compression and atomic rename are removed. ADR-0019 §5.
pub(crate) fn finalize_pending_rotations(memory_dir: &Path) -> Result<(), Error> {
    let read_dir = match std::fs::read_dir(memory_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    let mut pending = Vec::new();
    let mut stale_tmps = Vec::new();
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = ROTATED_LOG_SERIES
            .iter()
            .find_map(|series| name.strip_prefix(&format!("{series}.")))
        else {
            continue;
        };
        if let Some(num) = rest.strip_suffix(".jsonl.gz.tmp") {
            if num.parse::<u32>().is_ok() {
                stale_tmps.push(entry.path());
            }
            continue;
        }
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
    for tmp in stale_tmps {
        std::fs::remove_file(tmp)?;
    }
    for src in pending {
        let dst = src.with_extension("jsonl.gz");
        if dst.exists() {
            if gz_intact(&dst) {
                // Compression completed before the crash: keep the archive.
                std::fs::remove_file(&src)?;
            } else {
                // Truncated archive from a crash mid-gzip: the plain source
                // is complete, so rebuild the archive from it rather than
                // losing the tail of the log.
                std::fs::remove_file(&dst)?;
                gzip_file(&src, &dst)?;
                std::fs::remove_file(&src)?;
            }
            continue;
        }
        gzip_file(&src, &dst)?;
        std::fs::remove_file(&src)?;
    }
    Ok(())
}

/// Read a gzip stream to end-of-stream, returning false on any I/O or
/// format error (e.g. a truncated archive from a crash mid-compression).
fn gz_intact(path: &Path) -> bool {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut decoder = GzDecoder::new(file);
    let mut buf = [0u8; 64 * 1024];
    loop {
        match decoder.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

fn gzip_file(src: &Path, dst: &Path) -> Result<(), Error> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{BufReader, Read};
    let input = std::fs::File::open(src)?;
    let mut input = BufReader::new(input);
    // Compress into a temporary sibling and rename atomically so a crash
    // can never leave a truncated archive at `dst` for the recovery path to
    // prefer over the complete plain source.
    let mut tmp_name = dst.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let output = std::fs::File::create(&tmp)?;
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
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// Return whether an active JSONL series currently exceeds a rotation limit.
fn log_rotation_needed(log_path: &Path) -> Result<bool, Error> {
    let metadata = match std::fs::metadata(log_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.len() >= LOG_ROTATE_MAX_BYTES {
        return Ok(true);
    }
    Ok(count_lines(log_path)? >= LOG_ROTATE_MAX_LINES)
}

/// Rotate `{series}.jsonl` if it exceeds either size or line threshold.
/// Runs only at session boundaries (called from `Store::open`). ADR-0019 §5.
pub(crate) fn maybe_rotate_log(memory_dir: &Path, series: &str) -> Result<(), Error> {
    let log_path = memory_dir.join(format!("{series}.jsonl"));
    if !log_rotation_needed(&log_path)? {
        return Ok(());
    }
    // Rotation is opportunistic: a held lock means a writer is mid-append
    // (shared) or replay, vacuum or another rotation is running. Skip and
    // retry at the next open rather than failing `Store::open`.
    let _lock = match AverLock::acquire(memory_dir) {
        Ok(lock) => lock,
        Err(Error::LockHeld { .. }) => return Ok(()),
        Err(err) => return Err(err),
    };
    // Another process may have rotated after the optimistic check but before
    // this process acquired the lock. Recheck before renaming the active log.
    if !log_rotation_needed(&log_path)? {
        return Ok(());
    }
    let n = next_rotation_index(memory_dir, series)?;
    let intermediate = memory_dir.join(format!("{series}.{n}.jsonl"));
    std::fs::rename(&log_path, &intermediate)?;
    let target = memory_dir.join(format!("{series}.{n}.jsonl.gz"));
    gzip_file(&intermediate, &target)?;
    std::fs::remove_file(&intermediate)?;
    // Touch a fresh empty active log.
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    Ok(())
}

#[cfg(test)]
mod tail_tests {
    use super::last_line;

    #[test]
    fn last_line_reads_tail_without_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        assert_eq!(last_line(&path).unwrap(), None);
        std::fs::write(&path, "").unwrap();
        assert_eq!(last_line(&path).unwrap(), None);
        std::fs::write(&path, "only\n").unwrap();
        assert_eq!(last_line(&path).unwrap().as_deref(), Some("only"));
        // A line longer than one read block, followed by a short last line
        // and trailing blank lines.
        let long = "x".repeat(200 * 1024);
        std::fs::write(&path, format!("{long}\nlast\n\n")).unwrap();
        assert_eq!(last_line(&path).unwrap().as_deref(), Some("last"));
        // Last line itself longer than one block.
        std::fs::write(&path, format!("first\n{long}")).unwrap();
        assert_eq!(last_line(&path).unwrap().as_deref(), Some(long.as_str()));
    }
}

#[cfg(test)]
mod record_tests {
    use super::{ConfidenceChange, LogRecord};

    /// One literal line per kind, in the exact shape the live writer emits
    /// today (copied from real logs where available). Parsing must succeed
    /// and re-serialising must reproduce the bytes.
    const CURRENT_LINES: [&str; 15] = [
        r#"{"kind":"add_claim","ts":1778359774,"claim_id":1,"subject":"vasudev-core","predicate":"is_about","object":"a dual-mission project","source":"User asked to save project summary","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9,"provenance":"EXTRACTED","scope":"global"}"#,
        r#"{"kind":"add_hyperedge","ts":1,"hyperedge_id":7,"predicate":"deployment","provenance":"USER_ASSERTED","confidence":0.87,"source_refs":["deploy-log"],"participants":[{"role":"service","entity":"api"},{"role":"environment","entity":"prod"}]}"#,
        r#"{"kind":"record_event","ts":1778443106,"event_id":1,"session_id":"aver-cap-test-2026-05-10","event_kind":"user_message","payload":"TEST: aver-cli is built in Rust using clap as the argument parser.","source":"capability test","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","scope":"global"}"#,
        r#"{"kind":"record_observation","ts":1778443114,"observation_id":"58c04e5a562c","session_id":"aver-cap-test-2026-05-10","content":"TEST observation: aver-cli main.rs imports clap.","relevance":"low","source_event_ids":[1],"agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","derivation":"Inferred from reading main.rs lines 1-4.","scope":"global"}"#,
        r#"{"kind":"prune_observations","ts":5,"prune_marker_id":"a1b2c3d4e5f6","session_id":"s1","pruned_observation_ids":["58c04e5a562c","0000000000000001"]}"#,
        r#"{"kind":"retire_claim","ts":2,"claim_id":1,"reason":"superseded by docs"}"#,
        r#"{"kind":"add_contradiction","ts":3,"contradiction_id":9,"claim_id":1,"reason":"outdated dependency","new_claim_id":null}"#,
        r#"{"kind":"propose_candidate_claim","ts":2,"candidate_id":4,"event_id":1,"subject":"alpha","predicate":"depends_on","object":"beta","scope":"global"}"#,
        r#"{"kind":"promote_candidate_claim","ts":3,"candidate_id":4,"claim_id":12,"decision":{"model":"tev1:4b","choice":"promote"}}"#,
        r#"{"kind":"reject_candidate_claim","ts":4,"candidate_id":5,"reason":"not useful"}"#,
        r#"{"kind":"supersede_claims","ts":6,"claim_ids":[2,3]}"#,
        r#"{"kind":"decay_confidence","ts":7,"changes":[{"claim_id":1,"confidence":0.85},{"claim_id":2,"confidence":0.0}]}"#,
        r#"{"kind":"merge_source_refs","ts":8,"claim_id":1,"source_refs":["src1","src2"],"promote":true}"#,
        r#"{"kind":"add_vector_chunk","ts":9,"chunk_id":20,"claim_id":1,"text":"alpha depends_on beta","embedding_model":"test-model","embedding":[0.5,0.25,-1.0]}"#,
        r#"{"kind":"embed_vector_chunk","ts":10,"chunk_id":20,"embedding":[0.5,0.25,-1.0]}"#,
    ];

    #[test]
    fn current_lines_parse_and_round_trip_byte_for_byte() {
        for line in CURRENT_LINES {
            let record: LogRecord =
                serde_json::from_str(line).unwrap_or_else(|err| panic!("{line}: {err}"));
            assert_eq!(serde_json::to_string(&record).unwrap(), line);
        }
    }

    #[test]
    fn kinds_are_complete() {
        let mut parsed: Vec<&str> = CURRENT_LINES
            .iter()
            .map(|line| serde_json::from_str::<LogRecord>(line).unwrap().kind())
            .collect();
        parsed.sort_unstable();
        let mut kinds = LogRecord::KINDS.to_vec();
        kinds.sort_unstable();
        assert_eq!(parsed, kinds);
        for line in CURRENT_LINES {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let record: LogRecord = serde_json::from_str(line).unwrap();
            assert_eq!(value["kind"], record.kind());
        }
    }

    #[test]
    fn legacy_lines_without_optional_fields_parse_with_old_defaults() {
        // Pre-scope, pre-provenance claim line (the oldest shape in ~/.aver).
        let record: LogRecord = serde_json::from_str(
            r#"{"kind":"add_claim","ts":1778359774,"claim_id":1,"subject":"a","predicate":"p","object":"o","source":"s","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9}"#,
        )
        .unwrap();
        let LogRecord::AddClaim {
            provenance, scope, ..
        } = &record
        else {
            panic!("{record:?}")
        };
        assert_eq!(provenance.as_deref(), None);
        assert_eq!(scope, "global");

        // Pre-scope event, observation and candidate lines.
        for line in [
            r#"{"kind":"record_event","ts":1,"event_id":1,"session_id":"s","event_kind":"note","payload":"p","source":"s","agent_id":"local","agent_kind":"HUMAN"}"#,
            r#"{"kind":"record_observation","ts":2,"observation_id":"ob1","session_id":"s","content":"c","relevance":"high","source_event_ids":[1],"agent_id":"local","agent_kind":"HUMAN","derivation":"d"}"#,
            r#"{"kind":"propose_candidate_claim","ts":2,"candidate_id":1,"event_id":1,"subject":"a","predicate":"p","object":"o"}"#,
        ] {
            let record: LogRecord = serde_json::from_str(line).unwrap();
            let scope = match &record {
                LogRecord::RecordEvent { scope, .. }
                | LogRecord::RecordObservation { scope, .. }
                | LogRecord::ProposeCandidateClaim { scope, .. } => scope,
                other => panic!("{other:?}"),
            };
            assert_eq!(scope, "global");
        }

        // Promote/reject without the ADR-0024 decision field.
        let record: LogRecord = serde_json::from_str(
            r#"{"kind":"promote_candidate_claim","ts":3,"candidate_id":1,"claim_id":1}"#,
        )
        .unwrap();
        assert!(matches!(
            record,
            LogRecord::PromoteCandidateClaim { decision: None, .. }
        ));
    }

    #[test]
    fn unknown_kind_and_unknown_fields() {
        assert!(serde_json::from_str::<LogRecord>(r#"{"kind":"no_such_kind","ts":1}"#).is_err());
        // Unknown fields are ignored so older binaries can read newer logs.
        let record: LogRecord = serde_json::from_str(
            r#"{"kind":"retire_claim","ts":2,"claim_id":1,"reason":"r","future_field":true}"#,
        )
        .unwrap();
        assert_eq!(record.kind(), "retire_claim");
    }

    /// The log is the source of truth, so floats must come back bit-exact:
    /// a decayed confidence like 1.6740341892093765e-44 or an embedding
    /// component must not drift by an ulp through JSON.
    #[test]
    fn floats_round_trip_bit_exact() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let embedding: Vec<f32> = (0..2048)
            .map(|_| f32::from_bits((next() >> 32) as u32))
            .filter(|f| f.is_finite())
            .collect();
        let changes: Vec<ConfidenceChange> = (0..2048)
            .map(|i| ConfidenceChange {
                claim_id: i,
                confidence: f64::from_bits(next()),
            })
            .filter(|c| c.confidence.is_finite())
            .chain([ConfidenceChange {
                claim_id: -1,
                confidence: 1.6740341892093765e-44,
            }])
            .collect();
        let records = [
            LogRecord::EmbedVectorChunk {
                ts: 1,
                chunk_id: 1,
                embedding,
            },
            LogRecord::DecayConfidence { ts: 1, changes },
        ];
        for record in records {
            let line = serde_json::to_string(&record).unwrap();
            let parsed: LogRecord = serde_json::from_str(&line).unwrap();
            match (&record, &parsed) {
                (
                    LogRecord::EmbedVectorChunk { embedding: a, .. },
                    LogRecord::EmbedVectorChunk { embedding: b, .. },
                ) => assert!(
                    a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
                    "f32 drift"
                ),
                (
                    LogRecord::DecayConfidence { changes: a, .. },
                    LogRecord::DecayConfidence { changes: b, .. },
                ) => assert!(
                    a.iter()
                        .zip(b)
                        .all(|(x, y)| x.confidence.to_bits() == y.confidence.to_bits()),
                    "f64 drift"
                ),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn newest_minted_id_prefers_the_id_minted_last() {
        let record: LogRecord = serde_json::from_str(
            r#"{"kind":"promote_candidate_claim","ts":3,"candidate_id":4,"claim_id":12}"#,
        )
        .unwrap();
        assert_eq!(record.newest_minted_id(), Some(12));
        let record: LogRecord =
            serde_json::from_str(r#"{"kind":"retire_claim","ts":2,"claim_id":1,"reason":"r"}"#)
                .unwrap();
        assert_eq!(record.newest_minted_id(), None);
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::{LOG_ROTATE_MAX_LINES, lock_shared, maybe_rotate_log};
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::path::Path;

    /// Fill `{series}.jsonl` just past the line threshold with tiny lines.
    fn write_log_over_threshold(log_path: &Path) {
        let mut file = std::fs::File::create(log_path).unwrap();
        file.write_all(&b"{}\n".repeat(LOG_ROTATE_MAX_LINES as usize + 1))
            .unwrap();
    }

    /// Every byte of the series as replay would see it: rotated archives,
    /// then the active log.
    fn series_contents(memory_dir: &Path, series: &str) -> String {
        let mut out = String::new();
        for n in 1.. {
            let gz = memory_dir.join(format!("{series}.{n}.jsonl.gz"));
            if !gz.exists() {
                break;
            }
            flate2::read::GzDecoder::new(std::fs::File::open(gz).unwrap())
                .read_to_string(&mut out)
                .unwrap();
        }
        if let Ok(active) = std::fs::read_to_string(memory_dir.join(format!("{series}.jsonl"))) {
            out.push_str(&active);
        }
        out
    }

    #[test]
    fn append_in_flight_during_rotation_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("log.jsonl");
        write_log_over_threshold(&log_path);

        // Interleave rotation into `append_jsonl`'s open → write window: the
        // writer holds its shared lock and has opened the active log when
        // rotation runs.
        let lock = lock_shared(dir.path()).unwrap();
        let mut writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        maybe_rotate_log(dir.path(), "log").unwrap();
        writer.write_all(b"{\"marker\":true}\n").unwrap();
        writer.sync_data().unwrap();
        drop(writer);
        drop(lock);

        assert!(
            series_contents(dir.path(), "log").contains("{\"marker\":true}"),
            "a line appended while rotation ran must survive in the active log or an archive"
        );
    }

    #[test]
    fn rotation_is_skipped_while_a_writer_holds_the_shared_lock() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("log.jsonl");
        write_log_over_threshold(&log_path);
        let before = std::fs::read(&log_path).unwrap();
        let archive = dir.path().join("log.1.jsonl.gz");

        let lock = lock_shared(dir.path()).unwrap();
        maybe_rotate_log(dir.path(), "log").expect("a held lock must skip rotation, not fail");
        assert_eq!(std::fs::read(&log_path).unwrap(), before);
        assert!(!archive.exists(), "no archive while the lock is held");
        drop(lock);

        // Retried at the next open once the writer is done.
        maybe_rotate_log(dir.path(), "log").unwrap();
        assert!(archive.exists());
        assert_eq!(std::fs::metadata(&log_path).unwrap().len(), 0);
    }
}
