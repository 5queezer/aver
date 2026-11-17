//! Graph-first recall evaluation: twenty real operating questions over one
//! shared operating memory, run under three arms with a pinned clock:
//!
//! 1. `hybrid` — the current default: [`Store::recall_hybrid_claims`] with
//!    intent-classified [`HybridWeights::for_query`], recency off.
//! 2. `graph_first` — deterministic routing + bounded expansion, recency off.
//! 3. `graph_first_recency` — the same with the documented opt-in recency
//!    (ADR-0004 amendment: w = 0.08, tau = 90 days), which may only re-rank
//!    inside the bounded candidate set.
//!
//! Arms 2 and 3 run the real [`Store::recall_graph_first`]; when core
//! declines (no anchor, no edge, or a time relative to an event) the row records a marked hybrid
//! fallback, exactly as the MCP wrapper does.
//!
//! Determinism: the store is seeded by replaying `record_event` / `add_claim`
//! log lines with time-ordered ids (one source event per claim, linked via
//! `event_id`), consolidated and embedded with
//! [`HashedTokenEmbedder`], exactly as a
//! live store would be. The clock is the fixture's `now`.
//!
//! Metrics per question and arm:
//! - `answer_quality`: fraction of expected answer strings that exactly
//!   name a returned fact's subject or object (claim-id-based evidence
//!   metrics below stay exact).
//! - `evidence_coverage`: fraction of relevant fixture claims returned.
//! - `evidence_precision`: fraction of returned claims that are relevant.
//! - `payload_tokens`: whitespace tokens of the pretty-printed JSON facts
//!   payload the arm hands the caller (hybrid: triples; graph-first: facts
//!   plus the original events of selected claims). Response metadata
//!   (budget, latency) is excluded — this measures context cost for the
//!   consuming agent.
//! - `latency_micros`: wall time of the recall call; indicative, not
//!   benchmark-grade.
//! - `fell_back` / `fallback_rate`: graph-first arms only.
//! - `count_correct`: count-route questions where the arm reports a count.

use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use aver_core::retrieval::HybridWeights;
use aver_core::{AgentKind, ClaimStatus, RecallFilters, Store};
use serde::{Deserialize, Serialize};

use crate::recency::{HashedTokenEmbedder, TimedClaim};

/// 2024-01-01T00:00:00Z in Unix milliseconds: the ADR-0026 id epoch. Same
/// formula as the timed-fixture seeder so claim ids stay time-ordered.
const ID_EPOCH_MS: i64 = 1_704_067_200_000;

/// Documented opt-in recency for the `graph_first_recency` arm (ADR-0004
/// amendment, mirrored from tests/recency.rs).
pub const RECENCY_WEIGHT: f64 = 0.08;
pub const RECENCY_TAU_SECS: f64 = 90.0 * 86_400.0;

const EMBEDDING_MODEL: &str = "hashed-bag-of-tokens-256";
const VALID_ROUTES: [&str; 5] = ["single", "temporal", "intersection", "list", "count"];

/// One operating question with ground truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsQuery {
    pub query: String,
    /// Expected deterministic route: single | temporal | intersection | list | count.
    pub route: String,
    pub relevant_indices: Vec<usize>,
    pub top_k: usize,
    /// Answer strings a correct response must surface (normalized substring
    /// match against returned fact texts).
    pub expected_answers: Vec<String>,
    /// Expected count for count-route questions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_count: Option<usize>,
    /// The older competing claim, where one exists (temporal traps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Top-level fixture file. `now` is the pinned recall instant (Unix seconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsFixture {
    pub name: String,
    pub version: u32,
    pub description: String,
    pub now: i64,
    pub claims: Vec<TimedClaim>,
    pub queries: Vec<OpsQuery>,
}

/// Parse an ops fixture from a JSON string and validate its shape.
pub fn load_ops_fixture(json_str: &str) -> Result<OpsFixture> {
    let fixture: OpsFixture = serde_json::from_str(json_str)?;
    validate_ops_fixture(&fixture)?;
    Ok(fixture)
}

fn validate_ops_fixture(fixture: &OpsFixture) -> Result<()> {
    for (i, q) in fixture.queries.iter().enumerate() {
        if !VALID_ROUTES.contains(&q.route.as_str()) {
            bail!("query {i}: invalid route {:?}", q.route);
        }
        if q.expected_answers.is_empty() {
            bail!("query {i}: expected_answers must not be empty");
        }
        if q.top_k == 0 {
            bail!("query {i}: top_k must be positive");
        }
        for &idx in &q.relevant_indices {
            if idx >= fixture.claims.len() {
                bail!("query {i}: relevant index {idx} out of bounds");
            }
        }
        if q.route == "count" && q.expected_count.is_none() {
            bail!("query {i}: count route requires expected_count");
        }
        if let Some(stale) = q.stale_index
            && stale >= fixture.claims.len()
        {
            bail!("query {i}: stale index {stale} out of bounds");
        }
    }
    Ok(())
}

/// The three compared arms, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallArm {
    Hybrid,
    GraphFirst,
    GraphFirstRecency,
}

impl RecallArm {
    pub const ALL: [RecallArm; 3] = [
        RecallArm::Hybrid,
        RecallArm::GraphFirst,
        RecallArm::GraphFirstRecency,
    ];

    fn weights(self, query: &str) -> Result<HybridWeights> {
        let weights = HybridWeights::for_query(query);
        match self {
            RecallArm::GraphFirstRecency => {
                Ok(weights.with_recency(RECENCY_WEIGHT, RECENCY_TAU_SECS)?)
            }
            _ => Ok(weights),
        }
    }
}

/// Per-question, per-arm outcome.
#[derive(Debug, Clone, Serialize)]
pub struct ArmQueryOutcome {
    pub query: String,
    pub route: String,
    /// Fixture indices of the returned claims, in rank order.
    pub ranked_indices: Vec<usize>,
    pub answer_quality: f64,
    pub evidence_coverage: f64,
    pub evidence_precision: f64,
    pub payload_tokens: usize,
    pub latency_micros: u128,
    /// Graph-first arms only: routing produced no bounded candidate set and
    /// the caller fell back to hybrid.
    pub fell_back: Option<bool>,
    /// Core's reason when it fell back: "no_anchor" or "no_edges".
    pub fallback_reason: Option<String>,
    /// Count-route questions only, when the arm reports a count.
    pub count_correct: Option<bool>,
    /// The count the arm reported, when it reports one.
    pub count_value: Option<usize>,
}

/// Aggregate metrics over the arm's `ok` rows; `None` when nothing ran.
#[derive(Debug, Clone, Serialize)]
pub struct ArmAggregates {
    pub ok_questions: usize,
    pub mean_answer_quality: Option<f64>,
    pub mean_evidence_coverage: Option<f64>,
    pub mean_evidence_precision: Option<f64>,
    pub mean_payload_tokens: Option<f64>,
    pub total_payload_tokens: usize,
    pub mean_latency_micros: Option<f64>,
    pub p95_latency_micros: Option<u128>,
    /// Graph-first arms only.
    pub fallback_rate: Option<f64>,
}

/// One arm's full result.
#[derive(Debug, Clone, Serialize)]
pub struct ArmReport {
    pub arm: RecallArm,
    pub weights_summary: String,
    pub aggregates: ArmAggregates,
    pub questions: Vec<ArmQueryOutcome>,
}

/// The full three-arm comparison — the reproducible evaluation artifact.
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonReport {
    pub fixture_name: String,
    pub now: i64,
    pub claim_count: usize,
    pub event_count: usize,
    pub question_count: usize,
    pub recency_weight: f64,
    pub recency_tau_secs: f64,
    pub arms: Vec<ArmReport>,
}

/// What one recall call produced, normalized across arms for scoring.
struct RecallData {
    claim_ids: Vec<i64>,
    /// (subject, object) of every returned fact, for exact answer matching.
    fact_entities: Vec<(String, String)>,
    /// JSON facts payload as the caller receives it (token metric input).
    payload_json: String,
    fell_back: Option<bool>,
    fallback_reason: Option<String>,
    count: Option<usize>,
}

/// Run the real ADR-0033 entry point and normalize it for scoring.
/// `Ok(Err(reason))` means core declined (no anchor or no edge): the arm
/// falls back to hybrid, marked so the fallback rate and reason surface.
fn graph_first_recall(
    store: &Store,
    query: &str,
    top_k: usize,
    weights: HybridWeights,
    now: i64,
) -> Result<std::result::Result<RecallData, String>> {
    let options = aver_core::graph_first::GraphFirstOptions {
        top_k,
        weights,
        ..Default::default()
    };
    let result = store.recall_graph_first(query, &RecallFilters::default(), &options, now)?;
    let result = match result {
        Ok(result) => result,
        Err(fallback) => return Ok(Err(fallback.as_str().to_string())),
    };
    // The facts payload as the caller receives it: selected claims plus
    // their source events (response metadata like budget stays uncounted).
    let facts: Vec<serde_json::Value> = result
        .facts
        .iter()
        .map(|fact| {
            serde_json::json!({
                "claim_id": fact.claim.id,
                "subject": fact.claim.subject,
                "predicate": fact.claim.predicate,
                "object": fact.claim.object,
                "anchor": fact.anchor,
                "hop": fact.hop,
                "ts": fact.ts,
            })
        })
        .chain(result.events.iter().map(|event| {
            serde_json::json!({
                "event_id": event.id,
                "payload": event.payload,
            })
        }))
        .collect();
    Ok(Ok(RecallData {
        claim_ids: result.facts.iter().map(|fact| fact.claim.id).collect(),
        fact_entities: result
            .facts
            .iter()
            .map(|fact| (fact.claim.subject.clone(), fact.claim.object.clone()))
            .collect(),
        payload_json: serde_json::to_string_pretty(&facts)?,
        fell_back: Some(false),
        fallback_reason: None,
        count: result.count,
    }))
}

/// Hybrid fallback for declined graph-first queries, marked with the reason.
fn hybrid_fallback(
    store: &Store,
    embedder: &HashedTokenEmbedder,
    query: &str,
    top_k: usize,
    weights: HybridWeights,
    now: i64,
    reason: String,
) -> Result<RecallData> {
    let mut data = hybrid_recall(store, embedder, query, top_k, weights, now)?;
    data.fell_back = Some(true);
    data.fallback_reason = Some(reason);
    Ok(data)
}

fn hybrid_recall(
    store: &Store,
    embedder: &HashedTokenEmbedder,
    query: &str,
    top_k: usize,
    weights: HybridWeights,
    now: i64,
) -> Result<RecallData> {
    let retrieved = store.recall_hybrid_claims(query, embedder, top_k, weights, now)?;
    let payload: Vec<serde_json::Value> = retrieved
        .iter()
        .map(|claim| {
            serde_json::json!({
                "subject": claim.subject,
                "predicate": claim.predicate,
                "object": claim.object,
            })
        })
        .collect();
    Ok(RecallData {
        claim_ids: retrieved.iter().map(|claim| claim.id).collect(),
        fact_entities: retrieved
            .iter()
            .map(|claim| (claim.subject.clone(), claim.object.clone()))
            .collect(),
        payload_json: serde_json::to_string_pretty(&payload)?,
        fell_back: None,
        fallback_reason: None,
        count: None,
    })
}

/// (claim_id, event_id, fixture claim index) per seeded assertion.
type SeededIds = Vec<(i64, i64, usize)>;

/// Seed the fixture into a temp store by replaying log lines, exactly as a
/// live store ingests them: one `record_event` line per assertion, then the
/// `add_claim` line linked to it. Returns the store plus, per assertion, the
/// (claim_id, event_id, fixture claim index).
fn seed_ops_store(fixture: &OpsFixture) -> Result<(tempfile::TempDir, Store, SeededIds)> {
    let dir = tempfile::tempdir()?;

    let mut assertions: Vec<(i64, usize)> = Vec::new();
    for (idx, claim) in fixture.claims.iter().enumerate() {
        assertions.push((claim.ts, idx));
        for &ts in &claim.reasserted_at {
            if ts <= claim.ts {
                bail!(
                    "re-assertion at {ts} does not follow the write at {}",
                    claim.ts
                );
            }
            assertions.push((ts, idx));
        }
    }
    assertions.sort_by_key(|(ts, _)| *ts);

    let agent_kind = AgentKind::Human;
    let mut ids = Vec::with_capacity(assertions.len());
    let mut log = String::new();
    for (seq, (ts, claim_idx)) in assertions.iter().enumerate() {
        if *ts > fixture.now {
            bail!(
                "assertion at {ts} is after the fixture's now {}",
                fixture.now
            );
        }
        let claim = &fixture.claims[*claim_idx];
        let base = (ts * 1000 - ID_EPOCH_MS) << 12;
        let claim_id = base | seq as i64;
        // Events live in their own table; set bit 11 anyway so an event id
        // can never be mistaken for a claim id in a report row.
        let event_id = base | 0x800 | seq as i64;
        let event = serde_json::json!({
            "kind": "record_event",
            "ts": ts,
            "event_id": event_id,
            "session_id": "bench-graph-first-ops",
            "event_kind": "note",
            "payload": format!("{} {} {}", claim.subject, claim.predicate, claim.object),
            "source": format!("bench:{seq}"),
            "agent_id": "local",
            "agent_kind": agent_kind.as_str(),
            "scope": "global",
        });
        log.push_str(&event.to_string());
        log.push('\n');
        let line = serde_json::json!({
            "kind": "add_claim",
            "ts": ts,
            "claim_id": claim_id,
            "subject": claim.subject,
            "predicate": claim.predicate,
            "object": claim.object,
            "event_id": event_id,
            "source": format!("bench:{seq}"),
            "agent_id": "local",
            "agent_kind": agent_kind.as_str(),
            "confidence": agent_kind.default_provenance().policy_confidence(),
            "provenance": agent_kind.default_provenance().as_str(),
            "scope": "global",
        });
        log.push_str(&line.to_string());
        log.push('\n');
        ids.push((claim_id, event_id, *claim_idx));
    }
    std::fs::write(dir.path().join("log.jsonl"), log)?;
    aver_core::replay(dir.path(), false).context("replay ops fixture log")?;

    let store = Store::open(dir.path())?;
    store.consolidate()?;
    let embedder = HashedTokenEmbedder;
    for (claim_id, _, _) in &ids {
        if store.get_claim(*claim_id)?.status == ClaimStatus::Active {
            store.add_embedded_vector_chunk_for_claim(*claim_id, EMBEDDING_MODEL, &embedder)?;
        }
    }
    Ok((dir, store, ids))
}

/// Run one arm over all queries against the seeded store.
fn run_arm(
    fixture: &OpsFixture,
    store: &Store,
    id_index: &SeededIds,
    arm: RecallArm,
) -> Result<ArmReport> {
    let embedder = HashedTokenEmbedder;
    let mut questions = Vec::new();
    for q in &fixture.queries {
        let weights = arm.weights(&q.query)?;
        let started = Instant::now();
        let data = match arm {
            RecallArm::Hybrid => {
                hybrid_recall(store, &embedder, &q.query, q.top_k, weights, fixture.now)?
            }
            RecallArm::GraphFirst | RecallArm::GraphFirstRecency => {
                match graph_first_recall(store, &q.query, q.top_k, weights, fixture.now)? {
                    Ok(data) => data,
                    Err(reason) => hybrid_fallback(
                        store,
                        &embedder,
                        &q.query,
                        q.top_k,
                        weights,
                        fixture.now,
                        reason,
                    )?,
                }
            }
        };
        let latency_micros = started.elapsed().as_micros();

        let ranked_indices: Vec<usize> = data
            .claim_ids
            .iter()
            .filter_map(|id| {
                id_index
                    .iter()
                    .find(|(claim_id, _, _)| claim_id == id)
                    .map(|(_, _, idx)| *idx)
            })
            .collect();
        let relevant_found = ranked_indices
            .iter()
            .filter(|idx| q.relevant_indices.contains(idx))
            .count();
        // An expected answer is found when it exactly names a returned
        // fact's subject or object. Substring matching would credit
        // "stripe-api-v3" inside "stripe-api-v3-migration".
        let answers_found = q
            .expected_answers
            .iter()
            .filter(|answer| {
                data.fact_entities.iter().any(|(subject, object)| {
                    subject.eq_ignore_ascii_case(answer) || object.eq_ignore_ascii_case(answer)
                })
            })
            .count();

        questions.push(ArmQueryOutcome {
            query: q.query.clone(),
            route: q.route.clone(),
            ranked_indices,
            answer_quality: ratio(answers_found, q.expected_answers.len()),
            evidence_coverage: ratio(relevant_found, q.relevant_indices.len()),
            evidence_precision: if data.claim_ids.is_empty() {
                0.0
            } else {
                ratio(relevant_found, data.claim_ids.len())
            },
            payload_tokens: data.payload_json.split_whitespace().count(),
            latency_micros,
            fell_back: data.fell_back,
            fallback_reason: data.fallback_reason,
            count_correct: match (q.route.as_str(), q.expected_count, data.count) {
                ("count", Some(expected), Some(actual)) => Some(actual == expected),
                _ => None,
            },
            count_value: data.count,
        });
    }

    let ok: Vec<&ArmQueryOutcome> = questions.iter().collect();
    let mean = |value: fn(&ArmQueryOutcome) -> f64| -> Option<f64> {
        if ok.is_empty() {
            None
        } else {
            Some(ok.iter().map(|row| value(row)).sum::<f64>() / ok.len() as f64)
        }
    };
    let mut latencies: Vec<u128> = ok.iter().map(|row| row.latency_micros).collect();
    latencies.sort_unstable();
    let p95 = if latencies.is_empty() {
        None
    } else {
        let rank = (latencies.len() as f64 * 0.95).ceil() as usize;
        latencies.get(rank.saturating_sub(1)).copied()
    };
    let fallback_rate = match arm {
        RecallArm::Hybrid => None,
        _ if ok.is_empty() => None,
        _ => Some(ratio(
            ok.iter().filter(|row| row.fell_back == Some(true)).count(),
            ok.len(),
        )),
    };

    Ok(ArmReport {
        arm,
        weights_summary: match arm {
            RecallArm::Hybrid => "for_query(query), recency off".to_string(),
            RecallArm::GraphFirst => "for_query(query), recency off".to_string(),
            RecallArm::GraphFirstRecency => {
                format!("for_query(query) + recency w={RECENCY_WEIGHT}, tau={RECENCY_TAU_SECS}s")
            }
        },
        aggregates: ArmAggregates {
            ok_questions: ok.len(),
            mean_answer_quality: mean(|row| row.answer_quality),
            mean_evidence_coverage: mean(|row| row.evidence_coverage),
            mean_evidence_precision: mean(|row| row.evidence_precision),
            mean_payload_tokens: mean(|row| row.payload_tokens as f64),
            total_payload_tokens: ok.iter().map(|row| row.payload_tokens).sum(),
            mean_latency_micros: mean(|row| row.latency_micros as f64),
            p95_latency_micros: p95,
            fallback_rate,
        },
        questions,
    })
}

/// Run the full three-arm comparison for a fixture.
pub fn run_ops_fixture(fixture: &OpsFixture) -> Result<ComparisonReport> {
    let (_dir, store, id_index) = seed_ops_store(fixture)?;
    let event_count = id_index.len();
    let mut arms = Vec::new();
    for arm in RecallArm::ALL {
        arms.push(run_arm(fixture, &store, &id_index, arm)?);
    }
    Ok(ComparisonReport {
        fixture_name: fixture.name.clone(),
        now: fixture.now,
        claim_count: fixture.claims.len(),
        event_count,
        question_count: fixture.queries.len(),
        recency_weight: RECENCY_WEIGHT,
        recency_tau_secs: RECENCY_TAU_SECS,
        arms,
    })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
