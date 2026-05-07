//! MemoryAgentBench runner tests — strict TDD order.

const BASIC_FIXTURE: &str = include_str!("../../../eval/fixtures/basic_recall.json");
const CONFLICT_AND_NOISE_FIXTURE: &str =
    include_str!("../../../eval/fixtures/conflict_and_noise.json");
const AMBIGUOUS_SINGLE_TOKEN_FIXTURE: &str =
    include_str!("../../../eval/fixtures/ambiguous_single_token.json");

#[test]
fn fixture_deserializes_from_json() {
    let f = aver_eval::load_fixture(BASIC_FIXTURE).unwrap();
    assert_eq!(f.name, "basic_recall");
    assert_eq!(f.claims.len(), 5);
    assert_eq!(f.queries.len(), 3);
}

#[test]
fn runner_seeds_claims_into_store() {
    let f = aver_eval::load_fixture(BASIC_FIXTURE).unwrap();
    let metrics = aver_eval::run_fixture(&f).unwrap();
    // At least some queries returned results.
    assert!(metrics.query_results.iter().any(|q| q.retrieved_count > 0));
}

#[test]
fn recall_at_k_is_one_for_exact_query() {
    // Query "Alice" with relevant_indices=[0] — "Alice" appears as the object
    // of claim 0. recall_text("Alice") should find it.
    let f = aver_eval::load_fixture(BASIC_FIXTURE).unwrap();
    let metrics = aver_eval::run_fixture(&f).unwrap();
    let alice_result = metrics
        .query_results
        .iter()
        .find(|q| q.query == "Alice")
        .unwrap();
    assert_eq!(
        alice_result.recall_at_k, 1.0,
        "Expected perfect recall for exact object match 'Alice'"
    );
}

#[test]
fn no_relevant_empty_retrieval_scores_perfect_abstention() {
    let fixture = aver_eval::BenchFixture {
        name: "abstention".to_string(),
        description: "no relevant claims".to_string(),
        version: 1,
        claims: vec![aver_eval::FixtureClaim {
            subject: "project".to_string(),
            predicate: "language".to_string(),
            object: "Rust".to_string(),
        }],
        queries: vec![aver_eval::FixtureQuery {
            query: "Go".to_string(),
            relevant_indices: vec![],
            top_k: 5,
        }],
    };

    let metrics = aver_eval::run_fixture(&fixture).unwrap();

    assert_eq!(metrics.mean_recall_at_k, 1.0);
    assert_eq!(metrics.mean_precision_at_k, 1.0);
    assert_eq!(metrics.unsupported_claim_rate, 0.0);
}

#[test]
fn aggregate_metrics_weights_means_by_query_count() {
    let aggregate = aver_eval::aggregate_metrics(vec![
        aver_eval::BenchMetrics {
            fixture_name: "large".to_string(),
            mean_recall_at_k: 1.0,
            mean_precision_at_k: 1.0,
            unsupported_claim_rate: 0.0,
            query_results: vec![
                aver_eval::QueryResult {
                    query: "a".to_string(),
                    recall_at_k: 1.0,
                    precision_at_k: 1.0,
                    retrieved_count: 1,
                    relevant_found: 1,
                },
                aver_eval::QueryResult {
                    query: "b".to_string(),
                    recall_at_k: 1.0,
                    precision_at_k: 1.0,
                    retrieved_count: 1,
                    relevant_found: 1,
                },
                aver_eval::QueryResult {
                    query: "c".to_string(),
                    recall_at_k: 1.0,
                    precision_at_k: 1.0,
                    retrieved_count: 1,
                    relevant_found: 1,
                },
            ],
        },
        aver_eval::BenchMetrics {
            fixture_name: "small".to_string(),
            mean_recall_at_k: 0.0,
            mean_precision_at_k: 0.0,
            unsupported_claim_rate: 1.0,
            query_results: vec![aver_eval::QueryResult {
                query: "d".to_string(),
                recall_at_k: 0.0,
                precision_at_k: 0.0,
                retrieved_count: 1,
                relevant_found: 0,
            }],
        },
    ]);

    assert_eq!(aggregate.query_count, 4);
    assert_eq!(aggregate.mean_recall_at_k, 0.75);
    assert_eq!(aggregate.mean_precision_at_k, 0.75);
}

#[test]
fn aggregate_metrics_weights_unsupported_rate_by_retrieved_claims() {
    let aggregate = aver_eval::aggregate_metrics(vec![
        aver_eval::BenchMetrics {
            fixture_name: "many_retrieved".to_string(),
            mean_recall_at_k: 1.0,
            mean_precision_at_k: 1.0,
            unsupported_claim_rate: 0.0,
            query_results: vec![aver_eval::QueryResult {
                query: "a".to_string(),
                recall_at_k: 1.0,
                precision_at_k: 1.0,
                retrieved_count: 10,
                relevant_found: 10,
            }],
        },
        aver_eval::BenchMetrics {
            fixture_name: "one_bad".to_string(),
            mean_recall_at_k: 0.0,
            mean_precision_at_k: 0.0,
            unsupported_claim_rate: 1.0,
            query_results: vec![aver_eval::QueryResult {
                query: "b".to_string(),
                recall_at_k: 0.0,
                precision_at_k: 0.0,
                retrieved_count: 1,
                relevant_found: 0,
            }],
        },
    ]);

    assert!((aggregate.unsupported_claim_rate - (1.0 / 11.0)).abs() < f64::EPSILON);
}

#[test]
fn bench_metrics_serializes_to_json() {
    let f = aver_eval::load_fixture(BASIC_FIXTURE).unwrap();
    let metrics = aver_eval::run_fixture(&f).unwrap();
    let json = serde_json::to_string_pretty(&metrics).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["mean_recall_at_k"].is_number());
    assert!(v["unsupported_claim_rate"].is_number());
    assert!(v["query_results"].is_array());
}

#[test]
fn unsupported_claim_rate_bounded() {
    // unsupported_claim_rate must be between 0.0 and 1.0
    let f = aver_eval::load_fixture(BASIC_FIXTURE).unwrap();
    let metrics = aver_eval::run_fixture(&f).unwrap();
    assert!(metrics.unsupported_claim_rate >= 0.0);
    assert!(metrics.unsupported_claim_rate <= 1.0);
}

#[test]
fn ambiguous_single_answer_fixture_scopes_rust_query() {
    let f = aver_eval::load_fixture(AMBIGUOUS_SINGLE_TOKEN_FIXTURE).unwrap();
    let rust_query = f
        .queries
        .iter()
        .find(|query| query.relevant_indices == vec![0])
        .unwrap();

    assert_eq!(rust_query.query, "project Rust");
}

#[test]
fn conflict_and_noise_fixture_exercises_distractor_retrieval() {
    let f = aver_eval::load_fixture(CONFLICT_AND_NOISE_FIXTURE).unwrap();
    assert_eq!(f.name, "conflict_and_noise");
    assert!(f.claims.len() > 5);

    let metrics = aver_eval::run_fixture(&f).unwrap();
    assert!(metrics.mean_recall_at_k >= 0.5);
    assert!(metrics.mean_precision_at_k >= 0.5);
    assert!(metrics.unsupported_claim_rate >= 0.0);
    assert!(metrics.unsupported_claim_rate <= 1.0);
}
