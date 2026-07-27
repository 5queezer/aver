//! JSONL audit-log records, append helper, rotation, and the advisory lock.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

#[derive(Serialize)]
pub(crate) struct EventLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) event_id: i64,
    pub(crate) session_id: &'a str,
    pub(crate) event_kind: &'a str,
    pub(crate) payload: &'a str,
    pub(crate) source: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) agent_kind: &'a str,
    /// ADR-0021: written since the scope-replay fix; replay defaults a
    /// missing field to "global" so pre-fix log lines still parse.
    pub(crate) scope: &'a str,
}

#[derive(Serialize)]
pub(crate) struct ObservationLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) observation_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) content: &'a str,
    pub(crate) relevance: &'a str,
    pub(crate) source_event_ids: &'a [i64],
    pub(crate) agent_id: &'a str,
    pub(crate) agent_kind: &'a str,
    pub(crate) derivation: &'a str,
    /// ADR-0021: replay defaults a missing field to "global".
    pub(crate) scope: &'a str,
}

#[derive(Serialize)]
pub(crate) struct ObservationPruneLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) prune_marker_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) pruned_observation_ids: &'a [String],
}

#[derive(Serialize)]
pub(crate) struct LogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) claim_id: i64,
    pub(crate) subject: &'a str,
    pub(crate) predicate: &'a str,
    pub(crate) object: &'a str,
    pub(crate) source: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) agent_kind: &'a str,
    pub(crate) confidence: f64,
    /// Explicit provenance: deriving it from `agent_kind` at replay time
    /// misclassifies promoted candidates (an INFERRED candidate sourced by a
    /// HUMAN event replayed as USER_ASSERTED). Replay falls back to the
    /// agent_kind derivation for pre-fix log lines.
    pub(crate) provenance: &'a str,
    /// ADR-0021: replay defaults a missing field to "global".
    pub(crate) scope: &'a str,
}

/// ADR-0005: lifecycle transition for a claim retirement (ADR-0023). The
/// replay handler re-derives the `retired:<reason>` source_refs marker from
/// the replayed claim state, so the log only needs the reason.
#[derive(Serialize)]
pub(crate) struct RetireClaimLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) claim_id: i64,
    pub(crate) reason: &'a str,
}

/// ADR-0003/0005: a contradiction audit record. The contradiction id is
/// pre-allocated like claim ids so replay can pin it explicitly.
#[derive(Serialize)]
pub(crate) struct ContradictionLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) contradiction_id: i64,
    pub(crate) claim_id: i64,
    pub(crate) reason: &'a str,
    pub(crate) new_claim_id: Option<i64>,
}

/// ADR-0005: a staged candidate claim. Provenance and confidence are not
/// logged: proposal always uses the schema defaults (INFERRED, 0.45).
#[derive(Serialize)]
pub(crate) struct CandidateClaimLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) candidate_id: i64,
    pub(crate) event_id: i64,
    pub(crate) subject: &'a str,
    pub(crate) predicate: &'a str,
    pub(crate) object: &'a str,
    /// ADR-0021: replay defaults a missing field to "global".
    pub(crate) scope: &'a str,
}

/// ADR-0005: candidate promotion status flip. The promoted claim itself is
/// recorded by the paired `add_claim` line appended immediately before this
/// one in `promote_candidate_claim`.
#[derive(Serialize)]
pub(crate) struct PromoteCandidateLogEntry {
    pub(crate) kind: &'static str,
    pub(crate) ts: i64,
    pub(crate) candidate_id: i64,
    pub(crate) claim_id: i64,
}

#[derive(Serialize)]
pub(crate) struct RejectCandidateLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) candidate_id: i64,
    pub(crate) reason: &'a str,
}

/// ADR-0005: consolidation supersede outcome. Ids are captured before the
/// projection UPDATE so replay asserts the same lifecycle transition rather
/// than recomputing it from possibly-drifted state.
#[derive(Serialize)]
pub(crate) struct SupersedeLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) claim_ids: &'a [i64],
}

/// One (claim_id, confidence) pair inside a `decay_confidence` record.
#[derive(Serialize, Deserialize)]
pub(crate) struct ConfidenceChange {
    pub(crate) claim_id: i64,
    pub(crate) confidence: f64,
}

/// ADR-0005: confidence-decay outcome. Concrete post-decay values are logged
/// (not the decay formula) so the log stays the source of truth even if the
/// decay policy changes in a future binary.
#[derive(Serialize)]
pub(crate) struct DecayLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) changes: &'a [ConfidenceChange],
}

/// ADR-0005: source_refs merge outcome for one duplicate group. `promote`
/// mirrors the live merge's INFERRED->EXTRACTED promotion branch.
#[derive(Serialize)]
pub(crate) struct MergeSourceRefsLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) claim_id: i64,
    pub(crate) source_refs: &'a [String],
    pub(crate) promote: bool,
}

#[derive(Serialize)]
pub(crate) struct HyperedgeLogEntry<'a> {
    pub(crate) kind: &'a str,
    pub(crate) ts: i64,
    pub(crate) hyperedge_id: i64,
    pub(crate) predicate: &'a str,
    pub(crate) provenance: &'a str,
    pub(crate) confidence: f64,
    pub(crate) source_refs: &'a [String],
    pub(crate) participants: &'a [HyperedgeParticipantInput],
}

pub(crate) fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
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

/// Acquire the advisory `<memory_dir>/.lock` file (ADR-0019 §2/§5).
/// The OS lock is held by the retained file handle for the guard's lifetime.
pub struct AverLock {
    file: Option<File>,
}

impl AverLock {
    /// Acquire `<memory_dir>/.lock` without relying on PID-file liveness races.
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
        Ok(Self { file: Some(file) })
    }
}

impl Drop for AverLock {
    fn drop(&mut self) {
        // Closing the handle releases the advisory lock. Leave the marker path
        // in place so every contender locks the same inode.
        drop(self.file.take());
    }
}

/// Count newline-terminated lines in a file without loading it whole.
pub(crate) fn count_lines(path: &Path) -> std::io::Result<u64> {
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
pub(crate) fn next_rotation_index(memory_dir: &Path, series: &str) -> std::io::Result<u32> {
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
pub(crate) fn gz_intact(path: &Path) -> bool {
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

pub(crate) fn gzip_file(src: &Path, dst: &Path) -> Result<(), Error> {
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
    let _lock = AverLock::acquire(memory_dir)?;
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
