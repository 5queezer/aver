use aver_core::retrieval::HybridWeights;
use aver_eval::recency::{TimedFixture, load_timed_fixture, run_timed_fixture};

const RECENCY_FIXTURE: &str = include_str!("../../../eval/fixtures/recency_stale_over_fresh.json");

/// The documented opt-in setting (ADR-0004 amendment): `w = 0.08`,
/// `tau = 90` days.
const DOCUMENTED_WEIGHT: f64 = 0.08;
const DOCUMENTED_TAU_SECS: f64 = 90.0 * 86_400.0;

/// Top-ranked fixture claim index per query, each query ranked with its own
/// intent-classified alpha plus the given recency weight.
fn top_ranked(fixture: &TimedFixture, recency_weight: f64) -> Vec<usize> {
    fixture
        .queries
        .iter()
        .enumerate()
        .map(|(position, query)| {
            let weights = HybridWeights::for_query(&query.query)
                .with_recency(recency_weight, DOCUMENTED_TAU_SECS)
                .unwrap();
            let run = run_timed_fixture(fixture, weights).unwrap();
            assert_eq!(run.ranked_indices[position].len(), 1, "{}", query.query);
            run.ranked_indices[position][0]
        })
        .collect()
}

/// Baseline, and the default: without the recency term the named stale claim
/// takes the single result slot in every stale-versus-fresh query, while the
/// control query (which names the old object) is answered correctly.
#[test]
fn relevance_only_hybrid_recall_ranks_stale_claims_over_fresh_ones() {
    let fixture = load_timed_fixture(RECENCY_FIXTURE).unwrap();
    assert_eq!(
        fixture
            .queries
            .iter()
            .filter(|q| q.stale_index.is_some())
            .count(),
        3
    );

    for (query, top) in fixture.queries.iter().zip(top_ranked(&fixture, 0.0)) {
        let expected = query.stale_index.unwrap_or(query.relevant_indices[0]);
        assert_eq!(top, expected, "{}", query.query);
    }
}

/// With the documented opt-in weight the fresher claim wins every
/// stale-versus-fresh query and the control answer does not move.
#[test]
fn documented_recency_weight_ranks_fresh_claims_first_and_keeps_the_control() {
    let fixture = load_timed_fixture(RECENCY_FIXTURE).unwrap();

    for (query, top) in fixture
        .queries
        .iter()
        .zip(top_ranked(&fixture, DOCUMENTED_WEIGHT))
    {
        assert_eq!(top, query.relevant_indices[0], "{}", query.query);
    }
}

/// The weight is a trade-off, not a free win: too small leaves stale claims
/// on top, too large overrides a query that names the older claim.
#[test]
fn recency_weight_outside_the_working_range_fails_the_fixture() {
    let fixture = load_timed_fixture(RECENCY_FIXTURE).unwrap();
    let control = fixture
        .queries
        .iter()
        .position(|q| q.stale_index.is_none())
        .unwrap();

    let too_small = top_ranked(&fixture, 0.01);
    assert!(
        fixture
            .queries
            .iter()
            .zip(&too_small)
            .any(|(q, top)| q.stale_index == Some(*top))
    );
    let too_large = top_ranked(&fixture, 0.5);
    assert_ne!(
        too_large[control],
        fixture.queries[control].relevant_indices[0]
    );
}
