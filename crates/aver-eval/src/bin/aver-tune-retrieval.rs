use std::path::PathBuf;

use anyhow::Context;
use aver_eval::tuning::{suggest_next, TuningObservation, TuningSearchSpace};
use clap::Parser;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(about = "Suggest the next retrieval parameters for BEAM autoresearch")]
struct Args {
    /// JSONL file with observations containing top_k, alpha, and metric fields.
    #[arg(long)]
    observations: PathBuf,
    #[arg(long, default_value_t = 4)]
    top_k_min: usize,
    #[arg(long, default_value_t = 32)]
    top_k_max: usize,
    #[arg(long, default_value_t = 0.0)]
    alpha_min: f64,
    #[arg(long, default_value_t = 1.0)]
    alpha_max: f64,
    #[arg(long, default_value_t = 21)]
    alpha_steps: usize,
    #[arg(long, default_value_t = 0.10)]
    exploration: f64,
}

#[derive(Debug, Serialize)]
struct TuneReport {
    top_k: usize,
    alpha: f64,
    predicted_metric: f64,
    uncertainty: f64,
    acquisition: f64,
    autoresearch_env: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let observations = load_observations(&args.observations)?;
    let space = TuningSearchSpace {
        top_k_min: args.top_k_min,
        top_k_max: args.top_k_max,
        alpha_min: args.alpha_min,
        alpha_max: args.alpha_max,
        alpha_steps: args.alpha_steps,
        exploration: args.exploration,
    };
    let suggestion = suggest_next(&space, &observations)?;
    let report = TuneReport {
        top_k: suggestion.top_k,
        alpha: suggestion.alpha,
        predicted_metric: suggestion.predicted_metric,
        uncertainty: suggestion.uncertainty,
        acquisition: suggestion.acquisition,
        autoresearch_env: vec![
            "AVER_AUTORESEARCH_TARGET=beam".to_string(),
            format!("BEAM_TOP_K={}", suggestion.top_k),
            format!("BEAM_RETRIEVAL_ALPHA={:.3}", suggestion.alpha),
        ],
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn load_observations(path: &PathBuf) -> anyhow::Result<Vec<TuningObservation>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(idx, line)| {
            serde_json::from_str::<TuningObservation>(line)
                .with_context(|| format!("parse observation JSONL line {}", idx + 1))
        })
        .collect()
}
