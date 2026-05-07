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

#[test]
fn rank_candidates_breaks_score_ties_by_claim_id() {
    let ranked = rank_candidates(vec![
        RetrievalCandidate {
            claim_id: 2,
            score: 0.5,
        },
        RetrievalCandidate {
            claim_id: 1,
            score: 0.5,
        },
    ]);

    let ids: Vec<i64> = ranked.into_iter().map(|c| c.claim_id).collect();
    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn hybrid_weights_reject_alpha_outside_unit_interval() {
    assert!(HybridWeights::try_new(1.1).is_err());
    assert!(HybridWeights::try_new(-0.1).is_err());
}

#[test]
fn hybrid_weights_parse_alpha_override_from_string() {
    let weights: HybridWeights = "0.40".parse().unwrap();

    assert_eq!(weights.alpha, 0.40);
}

#[test]
fn hybrid_weights_parser_trims_alpha_override_whitespace() {
    let weights: HybridWeights = " 0.40 ".parse().unwrap();

    assert_eq!(weights.alpha, 0.40);
}

#[test]
fn retrieval_candidate_from_embeddings_uses_normalized_vector_score() {
    let candidate = RetrievalCandidate::from_embeddings(
        7,
        &[1.0, 0.0],
        &[1.0, 0.0],
        0.0,
        HybridWeights::default(),
    )
    .unwrap();

    assert_eq!(candidate.claim_id, 7);
    assert!((candidate.score - 0.65).abs() < f64::EPSILON);
}
