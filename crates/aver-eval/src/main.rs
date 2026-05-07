use std::{env, process};

use serde_json::json;

fn main() {
    if let Err(err) = run() {
        eprintln!("aver-eval: {err}");
        process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let fixture_paths: Vec<String> = env::args().skip(1).collect();
    if fixture_paths.is_empty() {
        return Err(anyhow::anyhow!(
            "usage: aver-eval <fixture.json> [fixture.json ...]"
        ));
    }

    let mut metrics = Vec::new();
    for fixture_path in fixture_paths {
        let fixture_json = std::fs::read_to_string(fixture_path)?;
        let fixture = aver_eval::load_fixture(&fixture_json)?;
        metrics.push(aver_eval::run_fixture(&fixture)?);
    }

    if metrics.len() == 1 {
        println!("{}", serde_json::to_string_pretty(&metrics[0])?);
        return Ok(());
    }

    let fixture_count = metrics.len();
    let query_count: usize = metrics
        .iter()
        .map(|metric| metric.query_results.len())
        .sum();
    let mean_recall_at_k = metrics
        .iter()
        .map(|metric| metric.mean_recall_at_k)
        .sum::<f64>()
        / fixture_count as f64;
    let mean_precision_at_k = metrics
        .iter()
        .map(|metric| metric.mean_precision_at_k)
        .sum::<f64>()
        / fixture_count as f64;
    let unsupported_claim_rate = metrics
        .iter()
        .map(|metric| metric.unsupported_claim_rate)
        .sum::<f64>()
        / fixture_count as f64;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "fixture_name": "aggregate",
            "fixture_count": fixture_count,
            "query_count": query_count,
            "mean_recall_at_k": mean_recall_at_k,
            "mean_precision_at_k": mean_precision_at_k,
            "unsupported_claim_rate": unsupported_claim_rate,
            "fixtures": metrics,
        }))?
    );
    Ok(())
}
