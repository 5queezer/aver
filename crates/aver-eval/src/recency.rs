//! Timed hybrid-recall fixtures: deterministic, offline, and with a time axis.
//!
//! [`run_fixture`](crate::run_fixture) seeds claims at wall-clock time and
//! ranks with keyword recall, so it cannot express "this claim is older than
//! that one". A timed fixture gives every claim a write timestamp (and
//! optional later re-assertions of the same triple) and ranks through the
//! real ADR-0004 hybrid path, at the fixture's own `now`, with a deterministic hashed bag-of-tokens
//! embedder.
//!
//! Timestamps reach the store the only way history can: as `add_claim` lines
//! in `log.jsonl` that are replayed into the projection. Nothing is UPDATEd
//! in place, so the fixture store is replay-equivalent by construction. A
//! re-assertion is a second `add_claim` line for the same triple, exactly
//! what a live re-assertion writes; consolidation then runs as it would in a
//! live store.

use anyhow::{Context as _, Result, bail};
use aver_core::retrieval::HybridWeights;
use aver_core::vector::{EmbeddingClient, EmbeddingError};
use aver_core::{AgentKind, ClaimStatus, Store};
use serde::{Deserialize, Serialize};

use crate::{BenchMetrics, QueryResult};

/// Dimension of [`HashedTokenEmbedder`] vectors. Deliberately not the
/// canonical ANN dimension, so ranking uses the exact cosine scan and equal
/// token overlap yields bit-equal scores.
pub const HASHED_EMBEDDING_DIM: usize = 256;

const EMBEDDING_MODEL: &str = "hashed-bag-of-tokens-256";

/// 2024-01-01T00:00:00Z in Unix milliseconds: the ADR-0026 id epoch.
const ID_EPOCH_MS: i64 = 1_704_067_200_000;

/// Deterministic offline embedder: lowercase alphanumeric tokens hashed
/// (FNV-1a) into [`HASHED_EMBEDDING_DIM`] buckets, L2-normalised. Similarity
/// is therefore token overlap with length normalisation, with no notion of
/// time or of which claim is "current".
#[derive(Debug, Clone, Copy, Default)]
pub struct HashedTokenEmbedder;

impl EmbeddingClient for HashedTokenEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vector = vec![0.0_f32; HASHED_EMBEDDING_DIM];
        for token in text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in token.to_ascii_lowercase().bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            vector[(hash % HASHED_EMBEDDING_DIM as u64) as usize] += 1.0;
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value /= norm;
            }
        }
        Ok(vector)
    }
}

/// A claim with its write time and any later re-assertions of the same
/// triple (Unix seconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedClaim {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasserted_at: Vec<i64>,
}

/// A query with ground-truth indices into the fixture's claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedQuery {
    pub query: String,
    pub relevant_indices: Vec<usize>,
    pub top_k: usize,
    /// The older competing claim that relevance alone ranks first. Absent on
    /// control queries, where relevance already picks the expected claim and
    /// recency must not change that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_index: Option<usize>,
    /// Why the stale claim wins on relevance alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Metrics plus, per query, the fixture indices of the retrieved claims in
/// rank order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedRun {
    pub metrics: BenchMetrics,
    pub ranked_indices: Vec<Vec<usize>>,
}

/// Top-level timed fixture file. `now` is the recall instant (Unix seconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedFixture {
    pub name: String,
    pub description: String,
    pub version: u32,
    pub now: i64,
    pub claims: Vec<TimedClaim>,
    pub queries: Vec<TimedQuery>,
}

/// Parse a timed fixture from a JSON string.
pub fn load_timed_fixture(json_str: &str) -> Result<TimedFixture> {
    serde_json::from_str(json_str).map_err(Into::into)
}

/// Run a timed fixture through hybrid recall with `weights`.
///
/// Steps:
/// 1. Write every assertion as an `add_claim` log line, in time order, with
///    a time-ordered id (ADR-0026), and replay the log into a temp store.
/// 2. Consolidate, as a live store does after ingest.
/// 3. Embed every ACTIVE claim with [`HashedTokenEmbedder`].
/// 4. Rank each query with `recall_hybrid_claims` and score it against the
///    fixture's relevant triples.
pub fn run_timed_fixture(fixture: &TimedFixture, weights: HybridWeights) -> Result<TimedRun> {
    let dir = tempfile::tempdir()?;

    let mut assertions: Vec<(i64, &TimedClaim)> = Vec::new();
    for claim in &fixture.claims {
        assertions.push((claim.ts, claim));
        for &ts in &claim.reasserted_at {
            if ts <= claim.ts {
                bail!(
                    "re-assertion at {ts} does not follow the write at {}",
                    claim.ts
                );
            }
            assertions.push((ts, claim));
        }
    }
    assertions.sort_by_key(|(ts, _)| *ts);

    let agent_kind = AgentKind::Human;
    let mut claim_ids = Vec::with_capacity(assertions.len());
    let mut log = String::new();
    for (seq, (ts, claim)) in assertions.iter().enumerate() {
        if *ts > fixture.now {
            bail!(
                "assertion at {ts} is after the fixture's now {}",
                fixture.now
            );
        }
        let claim_id = ((ts * 1000 - ID_EPOCH_MS) << 12) | seq as i64;
        claim_ids.push(claim_id);
        let line = serde_json::json!({
            "kind": "add_claim",
            "ts": ts,
            "claim_id": claim_id,
            "subject": claim.subject,
            "predicate": claim.predicate,
            "object": claim.object,
            "source": format!("bench:{seq}"),
            "agent_id": "local",
            "agent_kind": agent_kind.as_str(),
            "confidence": agent_kind.default_provenance().policy_confidence(),
            "provenance": agent_kind.default_provenance().as_str(),
            "scope": "global",
        });
        log.push_str(&line.to_string());
        log.push('\n');
    }
    std::fs::write(dir.path().join("log.jsonl"), log)?;
    aver_core::replay(dir.path(), false).context("replay timed fixture log")?;

    let store = Store::open(dir.path())?;
    store.consolidate()?;
    let embedder = HashedTokenEmbedder;
    for claim_id in claim_ids {
        if store.get_claim(claim_id)?.status == ClaimStatus::Active {
            store.add_embedded_vector_chunk_for_claim(claim_id, EMBEDDING_MODEL, &embedder)?;
        }
    }

    let mut query_results = Vec::new();
    let mut ranked_indices = Vec::new();
    let mut total_retrieved = 0;
    let mut total_non_relevant = 0;
    for fq in &fixture.queries {
        let retrieved =
            store.recall_hybrid_claims(&fq.query, &embedder, fq.top_k, weights, fixture.now)?;
        let ranked: Vec<usize> = retrieved
            .iter()
            .filter_map(|claim| {
                fixture.claims.iter().position(|fc| {
                    claim.subject == fc.subject
                        && claim.predicate == fc.predicate
                        && claim.object == fc.object
                })
            })
            .collect();
        let relevant_found = ranked
            .iter()
            .filter(|idx| fq.relevant_indices.contains(idx))
            .count();
        total_retrieved += retrieved.len();
        total_non_relevant += retrieved.len() - relevant_found;
        query_results.push(QueryResult {
            query: fq.query.clone(),
            recall_at_k: ratio(relevant_found, fq.relevant_indices.len()),
            precision_at_k: ratio(relevant_found, retrieved.len()),
            retrieved_count: retrieved.len(),
            relevant_found,
        });
        ranked_indices.push(ranked);
    }

    let n = query_results.len();
    let metrics = BenchMetrics {
        fixture_name: fixture.name.clone(),
        mean_recall_at_k: query_results.iter().map(|q| q.recall_at_k).sum::<f64>()
            / n.max(1) as f64,
        mean_precision_at_k: query_results.iter().map(|q| q.precision_at_k).sum::<f64>()
            / n.max(1) as f64,
        unsupported_claim_rate: ratio(total_non_relevant, total_retrieved),
        query_results,
    };
    Ok(TimedRun {
        metrics,
        ranked_indices,
    })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
