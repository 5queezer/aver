//! T16 — v0.2 HybridRAG starts with the hardcoded alpha from ADR-0004 before
//! adaptive classification exists.

use aver_core::ClaimWrite;
use aver_core::Store;
use aver_core::retrieval::{HybridWeights, RetrievalCandidate, rank_candidates, top_k_candidates};
use aver_core::vector::MockEmbeddingClient;

#[test]
fn default_hybrid_weights_use_hardcoded_v0_2_alpha() {
    let weights = HybridWeights::default();

    assert_eq!(weights.alpha, 0.65);
    assert_eq!(weights.vector_weight(), 0.65);
    assert_eq!(weights.graph_weight(), 0.35);
}

#[test]
fn adaptive_hybrid_weights_classifies_structural_and_semantic_queries() {
    assert_eq!(
        HybridWeights::for_query("what depends on PaymentGateway").alpha,
        0.35
    );
    assert_eq!(
        HybridWeights::for_query("summarize everything about OAuth migration").alpha,
        0.75
    );
    assert_eq!(HybridWeights::for_query("Stripe status").alpha, 0.65);
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

#[test]
fn rank_candidates_puts_nan_scores_last() {
    let ranked = rank_candidates(vec![
        RetrievalCandidate {
            claim_id: 1,
            score: f64::NAN,
        },
        RetrievalCandidate {
            claim_id: 2,
            score: 0.5,
        },
    ]);

    let ids: Vec<i64> = ranked.into_iter().map(|c| c.claim_id).collect();
    assert_eq!(ids, vec![2, 1]);
}

#[test]
fn hybrid_recall_alpha_zero_prefers_graph_match_over_vector_match() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let graph_match = store
        .add_claim(ClaimWrite::new(
            "auth_service",
            "calls",
            "validate_token",
            "test",
        ))
        .unwrap();
    let vector_match = store
        .add_claim(ClaimWrite::new(
            "billing",
            "mentions",
            "token validation",
            "test",
        ))
        .unwrap();
    store
        .add_vector_chunk_with_embedding(
            graph_match,
            "auth_service calls validate_token",
            "mock",
            &[0.0, 1.0],
        )
        .unwrap();
    store
        .add_vector_chunk_with_embedding(
            vector_match,
            "billing mentions token validation",
            "mock",
            &[1.0, 0.0],
        )
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let graph_first = store
        .recall_hybrid_claims(
            "auth_service",
            &client,
            2,
            HybridWeights::try_new(0.0).unwrap(),
        )
        .unwrap();

    assert_eq!(graph_first[0].id, graph_match);
}

#[test]
fn hybrid_recall_alpha_one_prefers_vector_match_over_graph_match() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let graph_match = store
        .add_claim(ClaimWrite::new(
            "auth_service",
            "calls",
            "validate_token",
            "test",
        ))
        .unwrap();
    let vector_match = store
        .add_claim(ClaimWrite::new(
            "billing",
            "mentions",
            "token validation",
            "test",
        ))
        .unwrap();
    store
        .add_vector_chunk_with_embedding(
            graph_match,
            "auth_service calls validate_token",
            "mock",
            &[0.0, 1.0],
        )
        .unwrap();
    store
        .add_vector_chunk_with_embedding(
            vector_match,
            "billing mentions token validation",
            "mock",
            &[1.0, 0.0],
        )
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let vector_first = store
        .recall_hybrid_claims(
            "auth_service",
            &client,
            2,
            HybridWeights::try_new(1.0).unwrap(),
        )
        .unwrap();

    assert_eq!(vector_first[0].id, vector_match);
}

#[test]
fn hybrid_recall_with_filters_applies_them_to_both_halves() {
    use aver_core::{ClaimStatus, RecallFilters, ScopeWalk};

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // Same text in two scopes, both with a vector chunk matching the query.
    let in_scope = store
        .add_claim(ClaimWrite::new("auth_service", "depends_on", "stripe_sdk", "s").scope("proj/a"))
        .unwrap();
    let out_of_scope = store
        .add_claim(ClaimWrite::new("auth_service", "depends_on", "stripe_sdk", "s").scope("proj/b"))
        .unwrap();
    // A vector-only candidate (no keyword overlap) that is low-confidence.
    let weak = store
        .add_claim(
            ClaimWrite::new("billing", "uses", "ledger", "s")
                .confidence(0.2)
                .scope("proj/a"),
        )
        .unwrap();
    for (id, text) in [
        (in_scope, "auth_service depends_on stripe_sdk"),
        (out_of_scope, "auth_service depends_on stripe_sdk"),
        (weak, "billing uses ledger"),
    ] {
        store
            .add_vector_chunk_with_embedding(id, text, "mock", &[1.0, 0.0])
            .unwrap();
    }
    let weights = HybridWeights::try_new(1.0).unwrap();
    let query = &[1.0_f32, 0.0];

    // Scope filter removes the out-of-scope twin on both halves.
    let scoped = store
        .recall_hybrid_claims_with_filters(
            "auth_service",
            query,
            5,
            weights,
            &RecallFilters {
                scope: "proj/a".to_string(),
                scope_walk: ScopeWalk::Exact,
                ..RecallFilters::default()
            },
        )
        .unwrap();
    let ids: Vec<i64> = scoped.iter().map(|claim| claim.id).collect();
    assert!(ids.contains(&in_scope) && ids.contains(&weak), "{ids:?}");
    assert!(!ids.contains(&out_of_scope), "{ids:?}");

    // min_confidence drops the vector-only candidate.
    let confident = store
        .recall_hybrid_claims_with_filters(
            "auth_service",
            query,
            5,
            weights,
            &RecallFilters {
                min_confidence: Some(0.5),
                ..RecallFilters::default()
            },
        )
        .unwrap();
    let ids: Vec<i64> = confident.iter().map(|claim| claim.id).collect();
    assert_eq!(ids, vec![in_scope, out_of_scope]);

    // Non-ACTIVE statuses are selectable: retire one claim, ask for it.
    store.retire_claim(out_of_scope, "stale").unwrap();
    let retired = store
        .recall_hybrid_claims_with_filters(
            "auth_service",
            query,
            5,
            weights,
            &RecallFilters {
                status: Some(ClaimStatus::Invalidated),
                ..RecallFilters::default()
            },
        )
        .unwrap();
    assert_eq!(
        retired.iter().map(|claim| claim.id).collect::<Vec<_>>(),
        vec![out_of_scope]
    );
    // And the default (ACTIVE) no longer returns it.
    let active = store
        .recall_hybrid_claims(
            "auth_service",
            &MockEmbeddingClient::new(vec![1.0, 0.0]),
            5,
            weights,
        )
        .unwrap();
    assert!(active.iter().all(|claim| claim.id != out_of_scope));
}
