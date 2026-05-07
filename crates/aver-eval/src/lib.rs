//! Deterministic, offline MemoryAgentBench runner for the Aver memory core.

use anyhow::Result;
use serde::{Deserialize, Serialize};

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
            if retrieved_count == 0 {
                1.0
            } else {
                0.0
            }
        } else {
            relevant_found as f64 / total_relevant as f64
        };
        let precision_at_k = if retrieved_count == 0 {
            if total_relevant == 0 {
                1.0
            } else {
                0.0
            }
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
