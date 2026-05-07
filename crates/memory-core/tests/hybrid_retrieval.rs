//! T16 — v0.2 HybridRAG starts with the hardcoded alpha from ADR-0004 before
//! adaptive classification exists.

use memory_core::retrieval::{
    HybridWeights, RetrievalCandidate, rank_candidates, top_k_candidates,
};

#[test]
fn default_hybrid_weights_use_hardcoded_v0_2_alpha() {
    let weights = HybridWeights::default();

    assert_eq!(weights.alpha, 0.65);
    assert_eq!(weights.vector_weight(), 0.65);
    assert_eq!(weights.graph_weight(), 0.35);
}

#[test]
fn hybrid_weights_blend_vector_and_graph_scores() {
    let weights = HybridWeights { alpha: 0.65 };

    let score = weights.blend(0.8, 0.2);

    assert!((score - 0.59).abs() < f64::EPSILON);
}

#[test]
fn retrieval_candidate_uses_hybrid_score_for_ordering_signal() {
    let weights = HybridWeights { alpha: 0.65 };
    let candidate = RetrievalCandidate::new(42, 0.8, 0.2, weights);

    assert_eq!(candidate.claim_id, 42);
    assert!((candidate.score - 0.59).abs() < f64::EPSILON);
}

#[test]
fn rank_candidates_orders_by_descending_hybrid_score() {
    let ranked = rank_candidates(vec![
        RetrievalCandidate {
            claim_id: 1,
            score: 0.2,
        },
        RetrievalCandidate {
            claim_id: 2,
            score: 0.9,
        },
        RetrievalCandidate {
            claim_id: 3,
            score: 0.5,
        },
    ]);

    let ids: Vec<i64> = ranked.into_iter().map(|c| c.claim_id).collect();
    assert_eq!(ids, vec![2, 3, 1]);
}

#[test]
fn top_k_candidates_returns_highest_scoring_limit() {
    let top = top_k_candidates(
        vec![
            RetrievalCandidate {
                claim_id: 1,
                score: 0.2,
            },
            RetrievalCandidate {
                claim_id: 2,
                score: 0.9,
            },
            RetrievalCandidate {
                claim_id: 3,
                score: 0.5,
            },
        ],
        2,
    );

    let ids: Vec<i64> = top.into_iter().map(|c| c.claim_id).collect();
    assert_eq!(ids, vec![2, 3]);
}
