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
