//! Retrieval scoring primitives for HybridRAG (ADR-0004).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HybridWeights {
    pub alpha: f64,
}

impl Default for HybridWeights {
    fn default() -> Self {
        Self { alpha: 0.65 }
    }
}

impl HybridWeights {
    pub fn vector_weight(self) -> f64 {
        self.alpha
    }

    pub fn graph_weight(self) -> f64 {
        1.0 - self.alpha
    }

    pub fn blend(self, vector_score: f64, graph_score: f64) -> f64 {
        self.vector_weight() * vector_score + self.graph_weight() * graph_score
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
}

pub fn rank_candidates(mut candidates: Vec<RetrievalCandidate>) -> Vec<RetrievalCandidate> {
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.claim_id.cmp(&b.claim_id))
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
