use aver_core::retrieval::HybridWeights;
use aver_eval::recency::{load_timed_fixture, run_timed_fixture};

const RECENCY_FIXTURE: &str = include_str!("../../../eval/fixtures/recency_stale_over_fresh.json");

/// Baseline: with relevance-only weights the stale claim takes the single
/// result slot in every stale-versus-fresh query, while the control query
/// (which names the old object) is answered correctly.
#[test]
fn relevance_only_hybrid_recall_ranks_stale_claims_over_fresh_ones() {
    let fixture = load_timed_fixture(RECENCY_FIXTURE).unwrap();

    for query in &fixture.queries {
        let metrics = run_timed_fixture(&fixture, HybridWeights::for_query(&query.query)).unwrap();
        let result = metrics
            .query_results
            .iter()
            .find(|result| result.query == query.query)
            .unwrap();
        let is_control = query.note.as_deref().unwrap().starts_with("Control");
        assert_eq!(result.retrieved_count, 1, "{}", query.query);
        assert_eq!(
            result.relevant_found,
            usize::from(is_control),
            "{}",
            query.query
        );
    }
}
