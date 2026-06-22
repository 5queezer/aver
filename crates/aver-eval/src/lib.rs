//! Deterministic, offline MemoryAgentBench runner for the Aver memory core.

pub mod beam;
pub mod prompt_assertions;
pub mod tuning;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QuerySuiteThresholds {
    pub max_drop_fraction: f64,
}

impl Default for QuerySuiteThresholds {
    fn default() -> Self {
        Self {
            max_drop_fraction: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuerySuiteRegressionFailure {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    pub drop_fraction: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuerySuiteRegressionReport {
    pub failed: bool,
    pub failures: Vec<QuerySuiteRegressionFailure>,
}

impl QuerySuiteRegressionReport {
    pub fn compare(
        thresholds: QuerySuiteThresholds,
        baseline_recall: f64,
        current_recall: f64,
        baseline_precision: f64,
        current_precision: f64,
        baseline_mrr: f64,
        current_mrr: f64,
    ) -> Self {
        let mut failures = Vec::new();
        for (metric, baseline, current) in [
            ("recall", baseline_recall, current_recall),
            ("precision", baseline_precision, current_precision),
            ("mrr", baseline_mrr, current_mrr),
        ] {
            if baseline <= 0.0 {
                continue;
            }
            let drop_fraction = (baseline - current) / baseline;
            if drop_fraction > thresholds.max_drop_fraction {
                failures.push(QuerySuiteRegressionFailure {
                    metric: metric.to_string(),
                    baseline,
                    current,
                    drop_fraction,
                });
            }
        }
        Self {
            failed: !failures.is_empty(),
            failures,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HallucinationCaseResult {
    pub id: String,
    pub memory_off_accurate: bool,
    pub memory_on_accurate: bool,
    pub memory_off_hallucinated: bool,
    pub memory_on_hallucinated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HallucinationRunReport {
    pub case_count: usize,
    pub memory_on_minus_off_accuracy: f64,
    pub memory_on_hallucination_rate: f64,
    pub memory_off_hallucination_rate: f64,
    pub memory_on_worse_case_ids: Vec<String>,
}

impl HallucinationRunReport {
    pub fn from_cases(cases: Vec<HallucinationCaseResult>) -> Self {
        let case_count = cases.len();
        let denominator = case_count.max(1) as f64;
        let memory_on_accuracy =
            cases.iter().filter(|case| case.memory_on_accurate).count() as f64 / denominator;
        let memory_off_accuracy =
            cases.iter().filter(|case| case.memory_off_accurate).count() as f64 / denominator;
        let memory_on_hallucination_rate = cases
            .iter()
            .filter(|case| case.memory_on_hallucinated)
            .count() as f64
            / denominator;
        let memory_off_hallucination_rate = cases
            .iter()
            .filter(|case| case.memory_off_hallucinated)
            .count() as f64
            / denominator;
        let memory_on_worse_case_ids = cases
            .iter()
            .filter(|case| case.memory_off_accurate && !case.memory_on_accurate)
            .map(|case| case.id.clone())
            .collect();

        Self {
            case_count,
            memory_on_minus_off_accuracy: memory_on_accuracy - memory_off_accuracy,
            memory_on_hallucination_rate,
            memory_off_hallucination_rate,
            memory_on_worse_case_ids,
        }
    }
}

/// A single claim to seed into the Store during a bench run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureClaim {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// A single query with ground-truth relevant indices into the fixture's claims list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureQuery {
    pub query: String,
    pub relevant_indices: Vec<usize>,
    pub top_k: usize,
}

/// Top-level fixture file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchFixture {
    pub name: String,
    pub description: String,
    pub version: u32,
    pub claims: Vec<FixtureClaim>,
    pub queries: Vec<FixtureQuery>,
}

/// Per-query result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query: String,
    pub recall_at_k: f64,
    pub precision_at_k: f64,
    pub retrieved_count: usize,
    pub relevant_found: usize,
}

/// Full run metrics — this is the JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchMetrics {
    pub fixture_name: String,
    pub mean_recall_at_k: f64,
    pub mean_precision_at_k: f64,
    /// Fraction of retrieved claims not relevant to any query.
    pub unsupported_claim_rate: f64,
    pub query_results: Vec<QueryResult>,
}

/// Aggregate metrics across multiple fixture runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub fixture_name: String,
    pub fixture_count: usize,
    pub query_count: usize,
    pub mean_recall_at_k: f64,
    pub mean_precision_at_k: f64,
    pub unsupported_claim_rate: f64,
    pub fixtures: Vec<BenchMetrics>,
}

/// Aggregate per-fixture metrics weighted by their query counts.
pub fn aggregate_metrics(fixtures: Vec<BenchMetrics>) -> AggregateMetrics {
    let fixture_count = fixtures.len();
    let query_count: usize = fixtures
        .iter()
        .map(|metric| metric.query_results.len())
        .sum();

    let weighted_sum = |value: fn(&BenchMetrics) -> f64| -> f64 {
        if query_count == 0 {
            0.0
        } else {
            fixtures
                .iter()
                .map(|metric| value(metric) * metric.query_results.len() as f64)
                .sum::<f64>()
                / query_count as f64
        }
    };

    let total_retrieved: usize = fixtures
        .iter()
        .flat_map(|metric| &metric.query_results)
        .map(|result| result.retrieved_count)
        .sum();
    let total_non_relevant: usize = fixtures
        .iter()
        .flat_map(|metric| &metric.query_results)
        .map(|result| result.retrieved_count.saturating_sub(result.relevant_found))
        .sum();
    let unsupported_claim_rate = if total_retrieved == 0 {
        0.0
    } else {
        total_non_relevant as f64 / total_retrieved as f64
    };

    AggregateMetrics {
        fixture_name: "aggregate".to_string(),
        fixture_count,
        query_count,
        mean_recall_at_k: weighted_sum(|metric| metric.mean_recall_at_k),
        mean_precision_at_k: weighted_sum(|metric| metric.mean_precision_at_k),
        unsupported_claim_rate,
        fixtures,
    }
}

/// Parse a fixture from a JSON string.
pub fn load_fixture(json_str: &str) -> Result<BenchFixture> {
    serde_json::from_str(json_str).map_err(Into::into)
}

/// Run a benchmark fixture against a temporary in-memory Store.
///
/// Steps:
/// 1. Open a temp Store.
/// 2. Seed all fixture claims.
/// 3. For each query, call `recall_text` and take the first `top_k` results.
/// 4. Match retrieved claims against the relevant indices from the fixture.
/// 5. Compute per-query and aggregate metrics.
pub fn run_fixture(fixture: &BenchFixture) -> Result<BenchMetrics> {
    let dir = tempfile::tempdir()?;
    let store = aver_core::Store::open(dir.path())?;

    // Seed all claims.
    for claim in &fixture.claims {
        store.add_claim(&claim.subject, &claim.predicate, &claim.object, "bench")?;
    }

    let mut query_results = Vec::new();
    let mut total_retrieved: usize = 0;
    let mut total_non_relevant: usize = 0;

    for fq in &fixture.queries {
        let all_retrieved = store.recall_text(&fq.query)?;
        // recall_text returns all matches; apply top_k cap.
        let retrieved: Vec<_> = all_retrieved.into_iter().take(fq.top_k).collect();
        let retrieved_count = retrieved.len();

        // Check each retrieved claim against the relevant indices.
        let mut relevant_found: usize = 0;
        let mut non_relevant_in_query: usize = 0;
        for claim in &retrieved {
            let matched = fq.relevant_indices.iter().any(|&idx| {
                if let Some(fc) = fixture.claims.get(idx) {
                    claim.subject == fc.subject
                        && claim.predicate == fc.predicate
                        && claim.object == fc.object
                } else {
                    false
                }
            });
            if matched {
                relevant_found += 1;
            } else {
                non_relevant_in_query += 1;
            }
        }

        let total_relevant = fq.relevant_indices.len();
        let recall_at_k = if total_relevant == 0 {
            if retrieved_count == 0 { 1.0 } else { 0.0 }
        } else {
            relevant_found as f64 / total_relevant as f64
        };
        let precision_at_k = if retrieved_count == 0 {
            if total_relevant == 0 { 1.0 } else { 0.0 }
        } else {
            let denom = retrieved_count.min(fq.top_k);
            relevant_found as f64 / denom as f64
        };

        total_retrieved += retrieved_count;
        total_non_relevant += non_relevant_in_query;

        query_results.push(QueryResult {
            query: fq.query.clone(),
            recall_at_k,
            precision_at_k,
            retrieved_count,
            relevant_found,
        });
    }

    let n = query_results.len() as f64;
    let mean_recall_at_k = if n == 0.0 {
        0.0
    } else {
        query_results.iter().map(|q| q.recall_at_k).sum::<f64>() / n
    };
    let mean_precision_at_k = if n == 0.0 {
        0.0
    } else {
        query_results.iter().map(|q| q.precision_at_k).sum::<f64>() / n
    };
    let unsupported_claim_rate = if total_retrieved == 0 {
        0.0
    } else {
        total_non_relevant as f64 / total_retrieved as f64
    };

    Ok(BenchMetrics {
        fixture_name: fixture.name.clone(),
        mean_recall_at_k,
        mean_precision_at_k,
        unsupported_claim_rate,
        query_results,
    })
}
