use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TuningObservation {
    pub top_k: usize,
    pub alpha: f64,
    pub metric: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningSearchSpace {
    pub top_k_min: usize,
    pub top_k_max: usize,
    pub alpha_min: f64,
    pub alpha_max: f64,
    pub alpha_steps: usize,
    pub exploration: f64,
}

impl Default for TuningSearchSpace {
    fn default() -> Self {
        Self {
            top_k_min: 4,
            top_k_max: 32,
            alpha_min: 0.0,
            alpha_max: 1.0,
            alpha_steps: 21,
            exploration: 0.10,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct TuningSuggestion {
    pub top_k: usize,
    pub alpha: f64,
    pub predicted_metric: f64,
    pub uncertainty: f64,
    pub acquisition: f64,
}

pub fn suggest_next(
    space: &TuningSearchSpace,
    observations: &[TuningObservation],
) -> Result<TuningSuggestion> {
    validate_space(space)?;
    if observations.is_empty() {
        return Ok(TuningSuggestion {
            top_k: 16.clamp(space.top_k_min, space.top_k_max),
            alpha: 0.65_f64.clamp(space.alpha_min, space.alpha_max),
            predicted_metric: 0.0,
            uncertainty: 1.0,
            acquisition: space.exploration,
        });
    }

    let mut best: Option<TuningSuggestion> = None;
    for top_k in space.top_k_min..=space.top_k_max {
        for alpha in alpha_grid(space) {
            if observations
                .iter()
                .any(|obs| obs.top_k == top_k && approx_eq(obs.alpha, alpha))
            {
                continue;
            }
            let (predicted_metric, uncertainty) = predict_metric(space, observations, top_k, alpha);
            let acquisition = predicted_metric + space.exploration * uncertainty;
            let suggestion = TuningSuggestion {
                top_k,
                alpha,
                predicted_metric,
                uncertainty,
                acquisition,
            };
            if best
                .as_ref()
                .is_none_or(|current| suggestion.acquisition > current.acquisition)
            {
                best = Some(suggestion);
            }
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("all retrieval tuning candidates have observations"))
}

fn validate_space(space: &TuningSearchSpace) -> Result<()> {
    if space.top_k_min == 0 || space.top_k_min > space.top_k_max {
        bail!("invalid top_k range")
    }
    if !(0.0..=1.0).contains(&space.alpha_min)
        || !(0.0..=1.0).contains(&space.alpha_max)
        || space.alpha_min > space.alpha_max
    {
        bail!("invalid alpha range")
    }
    if space.alpha_steps == 0 {
        bail!("alpha_steps must be greater than zero")
    }
    Ok(())
}

fn alpha_grid(space: &TuningSearchSpace) -> Vec<f64> {
    if space.alpha_steps == 1 {
        return vec![round_alpha(space.alpha_min)];
    }
    let step = (space.alpha_max - space.alpha_min) / (space.alpha_steps - 1) as f64;
    (0..space.alpha_steps)
        .map(|idx| round_alpha(space.alpha_min + idx as f64 * step))
        .collect()
}

fn predict_metric(
    space: &TuningSearchSpace,
    observations: &[TuningObservation],
    top_k: usize,
    alpha: f64,
) -> (f64, f64) {
    let mut weighted_sum = 0.0;
    let mut weight_sum = 0.0;
    for obs in observations {
        let distance = normalized_distance(space, obs.top_k, obs.alpha, top_k, alpha);
        let weight = (-distance * distance / 0.08).exp();
        weighted_sum += weight * obs.metric;
        weight_sum += weight;
    }
    let prior_mean =
        observations.iter().map(|obs| obs.metric).sum::<f64>() / observations.len() as f64;
    let prior_weight = 0.20;
    let mean = (weighted_sum + prior_mean * prior_weight) / (weight_sum + prior_weight);
    let uncertainty = 1.0 / (1.0 + weight_sum);
    (mean, uncertainty)
}

fn normalized_distance(
    space: &TuningSearchSpace,
    left_top_k: usize,
    left_alpha: f64,
    right_top_k: usize,
    right_alpha: f64,
) -> f64 {
    let top_k_span = (space.top_k_max - space.top_k_min).max(1) as f64;
    let alpha_span = (space.alpha_max - space.alpha_min).max(0.001);
    let top_k_delta = (left_top_k as f64 - right_top_k as f64) / top_k_span;
    let alpha_delta = (left_alpha - right_alpha) / alpha_span;
    (top_k_delta * top_k_delta + alpha_delta * alpha_delta).sqrt()
}

fn round_alpha(alpha: f64) -> f64 {
    (alpha * 1000.0).round() / 1000.0
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.0005
}
