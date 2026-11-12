//! ADR-0004 recency amendment: the opt-in `w * exp(-age / tau)` term, the
//! explicit recall instant, and the append-only re-verification record.
//!
//! Stores are seeded through `add_claim` log lines and replay, the only way
//! a claim gets a timestamp other than the wall clock.

use std::path::Path;

use aver_core::retrieval::{DEFAULT_RECENCY_TAU_SECS, HybridWeights};
use aver_core::vector::MockEmbeddingClient;
use aver_core::{ClaimStatus, Store, replay};

const DAY: i64 = 86_400;
const NOW: i64 = 1_793_491_200;

fn claim_line(ts: i64, claim_id: i64, subject: &str, object: &str) -> String {
    serde_json::json!({
        "kind": "add_claim", "ts": ts, "claim_id": claim_id,
        "subject": subject, "predicate": "uses", "object": object,
        "source": format!("src-{claim_id}"), "agent_id": "local", "agent_kind": "HUMAN",
        "confidence": 0.95, "provenance": "USER_ASSERTED", "scope": "global",
    })
    .to_string()
}

fn seeded_store(dir: &Path, lines: &[String]) -> Store {
    std::fs::write(dir.join("log.jsonl"), lines.join("\n") + "\n").unwrap();
    replay(dir, false).unwrap();
    Store::open(dir).unwrap()
}

fn log_lines(dir: &Path, kind: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("log.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|value| value["kind"] == kind)
        .collect()
}

fn recency(weight: f64) -> HybridWeights {
    HybridWeights::default()
        .with_recency(weight, DEFAULT_RECENCY_TAU_SECS)
        .unwrap()
}

#[test]
fn zero_weight_is_bit_identical_to_relevance_only_for_every_age() {
    let weights = HybridWeights::default();
    assert_eq!(weights.recency_weight, 0.0);
    let relevance = weights.blend(0.37, 0.81);
    for age in [
        0.0,
        1.0,
        -1.0,
        f64::MAX,
        f64::MIN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ] {
        let score = weights.blend_with_age(0.37, 0.81, age);
        assert_eq!(score.to_bits(), relevance.to_bits(), "age {age}");
    }
}

#[test]
fn recency_term_is_w_times_exp_of_negative_age_over_tau() {
    let weights = HybridWeights::default().with_recency(0.2, 100.0).unwrap();
    let relevance = weights.blend(0.5, 0.5);

    assert_eq!(weights.blend_with_age(0.5, 0.5, 0.0), relevance + 0.2);
    let one_tau = weights.blend_with_age(0.5, 0.5, 100.0);
    assert!((one_tau - (relevance + 0.2 * (-1.0_f64).exp())).abs() < 1e-12);
    // Future timestamps (clock skew) count as age zero, never as a bonus.
    assert_eq!(weights.blend_with_age(0.5, 0.5, -1e12), relevance + 0.2);
    assert_eq!(
        weights.blend_with_age(0.5, 0.5, f64::NEG_INFINITY),
        relevance + 0.2
    );
    // Very old claims lose the whole term without leaving a NaN behind.
    assert_eq!(weights.blend_with_age(0.5, 0.5, f64::MAX), relevance);
    assert_eq!(weights.blend_with_age(0.5, 0.5, f64::INFINITY), relevance);
    assert_eq!(weights.blend_with_age(0.5, 0.5, f64::NAN), relevance);
}

#[test]
fn with_recency_rejects_invalid_weight_and_tau() {
    let weights = HybridWeights::default();
    for weight in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        assert!(weights.with_recency(weight, 100.0).is_err(), "w {weight}");
    }
    for tau in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(weights.with_recency(0.1, tau).is_err(), "tau {tau}");
    }
    let opted_in = weights.with_recency(1.0, 1.0).unwrap();
    assert_eq!(opted_in.alpha, weights.alpha);
    assert_eq!(weights.with_recency(0.0, 1.0).unwrap().recency_weight, 0.0);
}

#[test]
fn verified_age_uses_last_verified_at_and_falls_back_to_write_ts() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(
        dir.path(),
        &[claim_line(NOW - 10 * DAY, 1, "api-gateway", "nginx")],
    );
    let mut claim = store.get_claim(1).unwrap();
    assert_eq!(claim.verified_age_secs(NOW), (10 * DAY) as f64);

    claim.last_verified_at = Some(NOW - DAY);
    assert_eq!(claim.verified_age_secs(NOW), DAY as f64);
    // Rows that never recorded a verification fall back to the write time.
    claim.last_verified_at = None;
    assert_eq!(claim.verified_age_secs(NOW), (10 * DAY) as f64);
    // A timestamp ahead of `now` is a negative age, clamped by the blend.
    assert_eq!(claim.verified_age_secs(NOW - 11 * DAY), -(DAY as f64));
    assert!(claim.verified_age_secs(i64::MIN).is_finite());
}

/// Two claims with identical relevance: relevance-only ranking breaks the
/// tie by the older id whatever `now` is; the opt-in term ranks the more
/// recently verified claim first, and `now` is the caller's, not the clock.
#[test]
fn recency_reorders_a_relevance_tie_only_when_opted_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(
        dir.path(),
        &[
            claim_line(NOW - 600 * DAY, 1, "api-gateway", "nginx"),
            claim_line(NOW - 20 * DAY, 2, "api-gateway", "envoy"),
        ],
    );
    for id in [1, 2] {
        store
            .add_vector_chunk_with_embedding(id, "chunk", "mock", &[1.0, 0.0])
            .unwrap();
    }
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);
    let ids = |weights: HybridWeights, now: i64| -> Vec<i64> {
        store
            .recall_hybrid_claims("api-gateway", &client, 2, weights, now)
            .unwrap()
            .into_iter()
            .map(|claim| claim.id)
            .collect()
    };

    for now in [0, NOW, i64::MAX, i64::MIN] {
        assert_eq!(ids(HybridWeights::default(), now), [1, 2], "now {now}");
    }
    assert_eq!(ids(recency(0.1), NOW), [2, 1]);
    // Seen from before either write, both ages clamp to zero: a tie again.
    assert_eq!(ids(recency(0.1), NOW - 700 * DAY), [1, 2]);
}

/// Re-asserting a triple writes a second row; consolidation keeps the oldest
/// row and must carry the re-assertion's time onto it through the log.
#[test]
fn consolidation_logs_reverification_and_replay_reproduces_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = NOW - 600 * DAY;
    let again = NOW - 12 * DAY;
    let store = seeded_store(
        dir.path(),
        &[
            claim_line(first, 1, "api-gateway", "envoy"),
            claim_line(NOW - 500 * DAY, 2, "api-gateway", "nginx"),
            claim_line(again, 3, "api-gateway", "envoy"),
        ],
    );
    assert_eq!(store.get_claim(1).unwrap().last_verified_at, Some(first));

    store.consolidate().unwrap();

    let survivor = store.get_claim(1).unwrap();
    assert_eq!(survivor.status, ClaimStatus::Active);
    assert_eq!(survivor.write_ts, first);
    assert_eq!(survivor.last_verified_at, Some(again));
    assert_eq!(store.get_claim(3).unwrap().status, ClaimStatus::Superseded);
    assert_eq!(
        store.get_claim(2).unwrap().last_verified_at,
        Some(NOW - 500 * DAY)
    );

    // The record holds ids and timestamps only: no claim content to filter.
    let records = log_lines(dir.path(), "reverify_claim");
    assert_eq!(records.len(), 1);
    let mut keys: Vec<&str> = records[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["claim_id", "kind", "ts", "verified_at"]);
    assert_eq!(records[0]["claim_id"], 1);
    assert_eq!(records[0]["verified_at"], again);

    // Consolidating again changes nothing and appends no further record.
    store.consolidate().unwrap();
    assert_eq!(log_lines(dir.path(), "reverify_claim").len(), 1);

    let live: Vec<_> = [1, 2, 3].map(|id| store.get_claim(id).unwrap()).into();
    store.close().unwrap();
    replay(dir.path(), true).unwrap();
    let replayed_store = Store::open(dir.path()).unwrap();
    let replayed: Vec<_> = [1, 2, 3]
        .map(|id| replayed_store.get_claim(id).unwrap())
        .into();
    assert_eq!(format!("{live:?}"), format!("{replayed:?}"));
}

#[test]
fn reverification_never_moves_last_verified_at_backwards() {
    let dir = tempfile::tempdir().unwrap();
    let written = NOW - 100 * DAY;
    let reverify = |ts: i64, verified_at: i64| {
        serde_json::json!({"kind": "reverify_claim", "ts": ts, "claim_id": 1, "verified_at": verified_at})
            .to_string()
    };
    let store = seeded_store(
        dir.path(),
        &[
            claim_line(written, 1, "api-gateway", "envoy"),
            reverify(NOW - 10 * DAY, NOW - 10 * DAY),
            // Out-of-order and duplicate records (merged logs, re-applied
            // lines) must be no-ops, including one before the write itself.
            reverify(NOW - 9 * DAY, NOW - 50 * DAY),
            reverify(NOW - 8 * DAY, written - DAY),
            reverify(NOW - 10 * DAY, NOW - 10 * DAY),
        ],
    );
    assert_eq!(
        store.get_claim(1).unwrap().last_verified_at,
        Some(NOW - 10 * DAY)
    );
}

#[test]
fn reverification_of_a_missing_claim_fails_strict_replay() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("log.jsonl"),
        r#"{"kind":"reverify_claim","ts":5,"claim_id":404,"verified_at":5}"#.to_string() + "\n",
    )
    .unwrap();
    let err = replay(dir.path(), false).unwrap_err().to_string();
    assert!(
        err.contains("reverify_claim references missing claim 404"),
        "{err}"
    );
}

/// A retired survivor is not silently refreshed by a later duplicate row.
#[test]
fn retired_claims_are_not_reverified() {
    let dir = tempfile::tempdir().unwrap();
    let first = NOW - 600 * DAY;
    let store = seeded_store(
        dir.path(),
        &[
            claim_line(first, 1, "api-gateway", "envoy"),
            serde_json::json!({"kind": "retire_claim", "ts": first + DAY, "claim_id": 1, "reason": "wrong"})
                .to_string(),
            claim_line(NOW - 12 * DAY, 3, "api-gateway", "envoy"),
        ],
    );
    store.consolidate().unwrap();
    assert_eq!(store.get_claim(1).unwrap().last_verified_at, Some(first));
    assert!(log_lines(dir.path(), "reverify_claim").is_empty());
}
