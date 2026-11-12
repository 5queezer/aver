//! Retrieval scoring primitives for HybridRAG (ADR-0004).

use crate::vector::normalized_cosine_score;

use std::num::ParseFloatError;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum HybridWeightsParseError {
    #[error("invalid alpha: {0}")]
    Float(#[from] ParseFloatError),
    #[error("alpha must be between 0.0 and 1.0")]
    OutOfRange,
    #[error("recency weight must be between 0.0 and 1.0")]
    RecencyWeightOutOfRange,
    #[error("recency tau must be a positive, finite number of seconds")]
    RecencyTauInvalid,
}

/// Default recency time constant: 90 days. Only used once a caller opts in
/// with a non-zero recency weight.
pub const DEFAULT_RECENCY_TAU_SECS: f64 = 90.0 * 86_400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HybridWeights {
    pub alpha: f64,
    /// Opt-in recency weight `w` (ADR-0004 amendment). `0.0`, the default,
    /// disables the term and leaves ranking exactly as relevance-only.
    pub recency_weight: f64,
    /// Recency time constant `tau` in seconds.
    pub recency_tau_secs: f64,
}

impl Default for HybridWeights {
    fn default() -> Self {
        Self::relevance_only(0.65)
    }
}

impl HybridWeights {
    pub fn try_new(alpha: f64) -> Result<Self, HybridWeightsParseError> {
        if (0.0..=1.0).contains(&alpha) {
            Ok(Self::relevance_only(alpha))
        } else {
            Err(HybridWeightsParseError::OutOfRange)
        }
    }

    const fn relevance_only(alpha: f64) -> Self {
        Self {
            alpha,
            recency_weight: 0.0,
            recency_tau_secs: DEFAULT_RECENCY_TAU_SECS,
        }
    }

    /// Opt in to the recency term: `weight` in `[0, 1]`, `tau_secs` positive
    /// and finite. `weight == 0.0` keeps relevance-only ranking.
    pub fn with_recency(self, weight: f64, tau_secs: f64) -> Result<Self, HybridWeightsParseError> {
        if !(0.0..=1.0).contains(&weight) {
            return Err(HybridWeightsParseError::RecencyWeightOutOfRange);
        }
        if !(tau_secs.is_finite() && tau_secs > 0.0) {
            return Err(HybridWeightsParseError::RecencyTauInvalid);
        }
        Ok(Self {
            recency_weight: weight,
            recency_tau_secs: tau_secs,
            ..self
        })
    }

    pub fn for_query(query: &str) -> Self {
        let lower = query.to_ascii_lowercase();
        let structural = [
            "depends on",
            "depends_on",
            "calls",
            "imports",
            "implements",
            "owned by",
            "owned_by",
            "related to",
            "connected to",
        ];
        if structural.iter().any(|needle| lower.contains(needle)) {
            return Self::relevance_only(0.35);
        }

        let semantic = [
            "summarize",
            "explain",
            "everything about",
            "overview",
            "context",
        ];
        if semantic.iter().any(|needle| lower.contains(needle)) {
            return Self::relevance_only(0.75);
        }

        Self::default()
    }

    pub fn vector_weight(self) -> f64 {
        self.alpha
    }

    pub fn graph_weight(self) -> f64 {
        1.0 - self.alpha
    }

    pub fn blend(self, vector_score: f64, graph_score: f64) -> f64 {
        self.vector_weight() * vector_score + self.graph_weight() * graph_score
    }

    /// [`blend`](Self::blend) plus the opt-in recency term
    /// `w * exp(-age / tau)`, where `age_secs` is the time since the claim
    /// was last verified. Negative ages (clock skew, future timestamps)
    /// count as zero. With `w == 0` the term is skipped, not multiplied, so
    /// the result is bit-identical to `blend` for every age.
    pub fn blend_with_age(self, vector_score: f64, graph_score: f64, age_secs: f64) -> f64 {
        let relevance = self.blend(vector_score, graph_score);
        if !(self.recency_weight > 0.0 && self.recency_tau_secs > 0.0) || age_secs.is_nan() {
            return relevance;
        }
        relevance + self.recency_weight * (-age_secs.max(0.0) / self.recency_tau_secs).exp()
    }
}

impl FromStr for HybridWeights {
    type Err = HybridWeightsParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_new(s.trim().parse()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalCandidate {
    pub claim_id: i64,
    pub score: f64,
}

impl RetrievalCandidate {
    pub fn new(claim_id: i64, vector_score: f64, graph_score: f64, weights: HybridWeights) -> Self {
        Self {
            claim_id,
            score: weights.blend(vector_score, graph_score),
        }
    }

    pub fn from_embeddings(
        claim_id: i64,
        query_embedding: &[f32],
        candidate_embedding: &[f32],
        graph_score: f64,
        weights: HybridWeights,
    ) -> Option<Self> {
        let vector_score = normalized_cosine_score(query_embedding, candidate_embedding)?;
        Some(Self::new(
            claim_id,
            f64::from(vector_score),
            graph_score,
            weights,
        ))
    }
}

pub fn rank_candidates(mut candidates: Vec<RetrievalCandidate>) -> Vec<RetrievalCandidate> {
    candidates.sort_by(|a, b| match (a.score.is_nan(), b.score.is_nan()) {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => b
            .score
            .total_cmp(&a.score)
            .then_with(|| a.claim_id.cmp(&b.claim_id)),
    });
    candidates
}

pub fn top_k_candidates(
    candidates: Vec<RetrievalCandidate>,
    top_k: usize,
) -> Vec<RetrievalCandidate> {
    let mut ranked = rank_candidates(candidates);
    ranked.truncate(top_k);
    ranked
}
