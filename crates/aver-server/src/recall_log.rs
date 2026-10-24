//! Opt-in, content-free recall-miss log for the roadmap Phase 4 agent trial.
//!
//! With `AVER_RECALL_MISS_LOG=1`, every MCP `recall` that returns no claims
//! (`kind: "miss"`), or whose top claim's confidence is below
//! `AVER_RECALL_WEAK_CONFIDENCE` (`kind: "weak"`, default 0.5), appends one
//! JSON line to `<memory_dir>/recall_misses.jsonl`.
//!
//! ADR-0009: the log is content-free by construction. [`RecallMissEvent`]
//! has no field that can hold query text, claim text, entity names or chunk
//! text: the query is reduced to two counts before an event exists, filters
//! are booleans, numbers or `&'static str` enum names, and the only strings
//! are the scope and the authenticated client id, both identifiers.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

use crate::tools::RecallView;

/// File name of the log inside the memory directory.
pub const RECALL_MISS_LOG_FILE: &str = "recall_misses.jsonl";

/// Default `AVER_RECALL_WEAK_CONFIDENCE`.
pub const DEFAULT_WEAK_CONFIDENCE: f64 = 0.5;

/// One logged recall. The serialized key set is pinned by a unit test, so a
/// new field has to be added deliberately.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallMissEvent {
    /// ISO-8601 UTC timestamp.
    pub ts: String,
    /// "miss" (no claims) or "weak" (top confidence below the threshold).
    pub kind: &'static str,
    /// Authenticated OAuth client id of the caller.
    pub agent_id: String,
    /// Query length in chars.
    pub query_len: usize,
    /// Count of whitespace-separated query tokens.
    pub query_tokens: usize,
    pub scope: String,
    pub scope_walk: &'static str,
    pub filters: RecallMissFilters,
    pub alpha: Option<f64>,
    /// "hybrid" or "keyword".
    pub retrieval: &'static str,
    pub top_k: usize,
    pub claims_returned: usize,
    pub chunks_returned: usize,
    pub top_confidence: Option<f64>,
    pub duration_ms: u64,
}

/// The recall filters, reduced to booleans, numbers and enum names.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallMissFilters {
    pub agent_id_set: bool,
    pub agent_kind: Option<&'static str>,
    pub predicate_set: bool,
    pub min_confidence: Option<f64>,
    /// A `ClaimStatus` name, or "any".
    pub status: &'static str,
}

/// What the log needs from the request, captured before the recall runs.
/// The query is already reduced to counts here.
#[derive(Debug, Clone)]
pub struct RecallRequestShape {
    pub query_len: usize,
    pub query_tokens: usize,
    pub scope: String,
    pub scope_walk: &'static str,
    pub filters: RecallMissFilters,
    pub top_k: usize,
}

impl RecallRequestShape {
    /// `(chars, whitespace tokens)` of a query.
    pub fn query_counts(query: &str) -> (usize, usize) {
        (query.chars().count(), query.split_whitespace().count())
    }
}

/// Appends recall-miss events to a JSONL file, opened on the first event.
pub struct RecallMissLog {
    path: PathBuf,
    weak_confidence: f64,
    file: Mutex<Option<File>>,
    warned: AtomicBool,
}

impl RecallMissLog {
    pub fn new(path: impl Into<PathBuf>, weak_confidence: f64) -> Self {
        Self {
            path: path.into(),
            weak_confidence,
            file: Mutex::new(None),
            warned: AtomicBool::new(false),
        }
    }

    /// `Some` only when `AVER_RECALL_MISS_LOG=1`.
    pub fn from_env(memory_dir: &Path) -> Option<Self> {
        Self::from_values(
            memory_dir,
            std::env::var("AVER_RECALL_MISS_LOG").ok().as_deref(),
            std::env::var("AVER_RECALL_WEAK_CONFIDENCE").ok().as_deref(),
        )
    }

    fn from_values(memory_dir: &Path, enabled: Option<&str>, weak: Option<&str>) -> Option<Self> {
        if enabled.map(str::trim) != Some("1") {
            return None;
        }
        let weak_confidence = weak
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| (0.0..=1.0).contains(value))
            .unwrap_or(DEFAULT_WEAK_CONFIDENCE);
        Some(Self::new(
            memory_dir.join(RECALL_MISS_LOG_FILE),
            weak_confidence,
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Classifies a finished recall; `None` means a hit, which is not logged.
    pub fn event(
        &self,
        shape: RecallRequestShape,
        view: &RecallView,
        agent_id: &str,
        duration: Duration,
    ) -> Option<RecallMissEvent> {
        let top_confidence = view.triples.first().map(|claim| claim.confidence);
        let kind = match top_confidence {
            None => "miss",
            Some(confidence) if confidence < self.weak_confidence => "weak",
            Some(_) => return None,
        };
        Some(RecallMissEvent {
            ts: now_iso8601(),
            kind,
            agent_id: agent_id.to_string(),
            query_len: shape.query_len,
            query_tokens: shape.query_tokens,
            scope: shape.scope,
            scope_walk: shape.scope_walk,
            filters: shape.filters,
            alpha: view.alpha,
            retrieval: if view.retrieval == "hybrid" {
                "hybrid"
            } else {
                "keyword"
            },
            top_k: shape.top_k,
            claims_returned: view.triples.len(),
            chunks_returned: view.chunks.len(),
            top_confidence,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// Logs the recall if it is a miss or weak hit. Never fails: a write
    /// error is reported to stderr once and the recall result stands.
    pub fn observe(
        &self,
        shape: RecallRequestShape,
        view: &RecallView,
        agent_id: &str,
        duration: Duration,
    ) {
        if let Some(event) = self.event(shape, view, agent_id, duration)
            && let Err(err) = self.append(&event)
            && !self.warned.swap(true, Ordering::Relaxed)
        {
            eprintln!(
                "aver-server: recall-miss log write to {} failed (further errors suppressed): {err}",
                self.path.display()
            );
        }
    }

    fn append(&self, event: &RecallMissEvent) -> anyhow::Result<()> {
        let line = serde_json::to_string(event)?;
        let mut guard = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("recall-miss log lock poisoned"))?;
        let file = match guard.as_mut() {
            Some(file) => file,
            None => guard.insert(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            ),
        };
        writeln!(file, "{line}")?;
        Ok(())
    }
}

fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .ok()
        .and_then(|ts| {
            ts.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::GraphView;

    fn shape() -> RecallRequestShape {
        RecallRequestShape {
            query_len: 12,
            query_tokens: 2,
            scope: "proj/aver".to_string(),
            scope_walk: "ancestors",
            filters: RecallMissFilters {
                agent_id_set: false,
                agent_kind: None,
                predicate_set: false,
                min_confidence: None,
                status: "ACTIVE",
            },
            top_k: 5,
        }
    }

    fn empty_view() -> RecallView {
        RecallView {
            triples: Vec::new(),
            chunks: Vec::new(),
            subgraph: GraphView {
                nodes: Vec::new(),
                edges: Vec::new(),
            },
            confidence_floor: 0.0,
            retrieval: "keyword".to_string(),
            alpha: None,
        }
    }

    #[test]
    fn serialized_event_has_exactly_the_documented_keys() {
        let log = RecallMissLog::new("unused", DEFAULT_WEAK_CONFIDENCE);
        let event = log
            .event(
                shape(),
                &empty_view(),
                "aver-client",
                Duration::from_millis(3),
            )
            .expect("empty recall is a miss");
        let value = serde_json::to_value(&event).unwrap();
        let object = value.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "agent_id",
                "alpha",
                "chunks_returned",
                "claims_returned",
                "duration_ms",
                "filters",
                "kind",
                "query_len",
                "query_tokens",
                "retrieval",
                "scope",
                "scope_walk",
                "top_confidence",
                "top_k",
                "ts",
            ]
        );
        let mut filter_keys: Vec<&str> = object["filters"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        filter_keys.sort_unstable();
        assert_eq!(
            filter_keys,
            [
                "agent_id_set",
                "agent_kind",
                "min_confidence",
                "predicate_set",
                "status",
            ]
        );
        assert_eq!(object["kind"], "miss");
        let ts = object["ts"].as_str().unwrap();
        assert!(ts.ends_with('Z') && ts.len() == 20, "ISO-8601 UTC: {ts}");
    }

    #[test]
    fn opt_in_requires_exactly_one() {
        let dir = Path::new("/mem");
        assert!(RecallMissLog::from_values(dir, None, None).is_none());
        assert!(RecallMissLog::from_values(dir, Some("0"), None).is_none());
        assert!(RecallMissLog::from_values(dir, Some("true"), None).is_none());
        let log = RecallMissLog::from_values(dir, Some("1"), None).unwrap();
        assert_eq!(log.path(), Path::new("/mem/recall_misses.jsonl"));
        assert_eq!(log.weak_confidence, DEFAULT_WEAK_CONFIDENCE);
        let log = RecallMissLog::from_values(dir, Some("1"), Some("0.8")).unwrap();
        assert_eq!(log.weak_confidence, 0.8);
        let log = RecallMissLog::from_values(dir, Some("1"), Some("7")).unwrap();
        assert_eq!(log.weak_confidence, DEFAULT_WEAK_CONFIDENCE);
    }

    #[test]
    fn query_counts_are_chars_and_whitespace_tokens() {
        assert_eq!(
            RecallRequestShape::query_counts("  héllo   world "),
            (16, 2)
        );
        assert_eq!(RecallRequestShape::query_counts(""), (0, 0));
    }
}
