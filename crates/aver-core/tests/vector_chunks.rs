//! T14 — vector chunk metadata can be written for a claim before sqlite-vss
//! indexing is wired in.

use aver_core::{
    Store,
    vector::{EmbeddingClient, EmbeddingError, MockEmbeddingClient},
};

struct FailingEmbeddingClient;

impl EmbeddingClient for FailingEmbeddingClient {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Err(EmbeddingError::Http("should not embed".to_string()))
    }
}

#[test]
fn add_vector_chunk_returns_chunk_id_for_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();

    let chunk_id = store
        .add_vector_chunk(
            claim_id,
            "auth_service depends_on stripe_sdk",
            "nomic-embed-text",
        )
        .expect("chunk metadata insert should succeed");

    assert_eq!(chunk_id, 1);
}

#[test]
fn get_vector_chunk_returns_claim_text_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let chunk_id = store
        .add_vector_chunk(
            claim_id,
            "auth_service depends_on stripe_sdk",
            "nomic-embed-text",
        )
        .unwrap();

    let chunk = store
        .get_vector_chunk(chunk_id)
        .expect("chunk metadata should be readable");

    assert_eq!(chunk.id, chunk_id);
    assert_eq!(chunk.claim_id, claim_id);
    assert_eq!(chunk.text, "auth_service depends_on stripe_sdk");
    assert_eq!(chunk.embedding_model, "nomic-embed-text");
}

#[test]
fn list_vector_chunks_for_claim_orders_by_chunk_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    store
        .add_vector_chunk(claim_id, "first chunk", "nomic-embed-text")
        .unwrap();
    store
        .add_vector_chunk(claim_id, "second chunk", "nomic-embed-text")
        .unwrap();

    let chunks = store
        .list_vector_chunks_for_claim(claim_id)
        .expect("chunks should be listed");

    let texts: Vec<String> = chunks.into_iter().map(|chunk| chunk.text).collect();
    assert_eq!(texts, vec!["first chunk", "second chunk"]);
}

#[test]
fn add_vector_chunk_for_claim_uses_canonical_claim_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();

    let chunk_id = store
        .add_vector_chunk_for_claim(claim_id, "nomic-embed-text")
        .unwrap();
    let chunk = store.get_vector_chunk(chunk_id).unwrap();

    assert_eq!(chunk.text, "auth_service depends_on stripe_sdk");
    assert_eq!(chunk.embedding_model, "nomic-embed-text");
}

#[test]
fn add_vector_chunk_with_embedding_persists_embedding_vector() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();

    let chunk_id = store
        .add_vector_chunk_with_embedding(
            claim_id,
            "auth_service depends_on stripe_sdk",
            "nomic-embed-text",
            &[0.1, 0.2, 0.3],
        )
        .unwrap();
    let chunk = store.get_vector_chunk(chunk_id).unwrap();

    assert_eq!(chunk.embedding, Some(vec![0.1, 0.2, 0.3]));
}

#[test]
fn add_embedded_vector_chunk_for_claim_uses_embedding_client() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let client = MockEmbeddingClient::new(vec![0.4, 0.5]);

    let chunk_id = store
        .add_embedded_vector_chunk_for_claim(claim_id, "nomic-embed-text", &client)
        .unwrap();
    let chunk = store.get_vector_chunk(chunk_id).unwrap();

    assert_eq!(chunk.text, "auth_service depends_on stripe_sdk");
    assert_eq!(chunk.embedding, Some(vec![0.4, 0.5]));
}

#[test]
fn rank_vector_chunks_by_embedding_returns_best_match_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let other_id = store
        .add_claim("billing_worker", "emits", "invoice_event", "test_session")
        .unwrap();
    let best_id = store
        .add_vector_chunk_with_embedding(claim_id, "auth", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    store
        .add_vector_chunk_with_embedding(other_id, "billing", "nomic-embed-text", &[0.0, 1.0])
        .unwrap();

    let ranked = store
        .rank_vector_chunks_by_embedding(&[1.0, 0.0], 1)
        .expect("ranking should score stored embeddings");

    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].id, best_id);
}

#[test]
fn recall_vector_chunks_embeds_query_with_client_before_ranking() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let best_id = store
        .add_vector_chunk_with_embedding(claim_id, "auth", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let ranked = store
        .recall_vector_chunks("auth query", &client, 1)
        .expect("query embedding should rank stored vector chunks");

    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].id, best_id);
}

#[test]
fn recall_vector_claims_returns_claims_for_ranked_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    store
        .add_vector_chunk_with_embedding(claim_id, "auth", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let claims = store
        .recall_vector_claims("auth query", &client, 1)
        .expect("vector recall should return claims, not chunk metadata");

    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, claim_id);
    assert_eq!(claims[0].subject, "auth_service");
}

#[test]
fn recall_vector_claims_deduplicates_multiple_chunks_for_same_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    store
        .add_vector_chunk_with_embedding(claim_id, "auth one", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    store
        .add_vector_chunk_with_embedding(claim_id, "auth two", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let claims = store
        .recall_vector_claims("auth query", &client, 8)
        .expect("vector recall should deduplicate claims");

    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, claim_id);
}

#[test]
fn recall_vector_claims_fills_top_k_with_distinct_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let auth_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let billing_id = store
        .add_claim("billing_worker", "emits", "invoice_event", "test_session")
        .unwrap();
    store
        .add_vector_chunk_with_embedding(auth_id, "auth one", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    store
        .add_vector_chunk_with_embedding(auth_id, "auth two", "nomic-embed-text", &[1.0, 0.0])
        .unwrap();
    store
        .add_vector_chunk_with_embedding(billing_id, "billing", "nomic-embed-text", &[0.8, 0.2])
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let claims = store
        .recall_vector_claims("auth query", &client, 2)
        .expect("vector recall should fill top_k with distinct claims");

    let ids: Vec<i64> = claims.into_iter().map(|claim| claim.id).collect();
    assert_eq!(ids, vec![auth_id, billing_id]);
}

#[test]
fn recall_hybrid_claims_falls_back_to_text_when_vector_index_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0]);

    let claims = store
        .recall_hybrid_claims("stripe", &client, 8)
        .expect("hybrid recall should use text fallback when no vectors exist");

    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, claim_id);
}

#[test]
fn recall_hybrid_claims_top_k_zero_does_not_embed_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claims = store
        .recall_hybrid_claims("anything", &FailingEmbeddingClient, 0)
        .expect("top_k=0 should short-circuit before embedding");

    assert!(claims.is_empty());
}

#[test]
fn recall_vector_claims_top_k_zero_does_not_embed_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claims = store
        .recall_vector_claims("anything", &FailingEmbeddingClient, 0)
        .expect("top_k=0 should short-circuit before embedding");

    assert!(claims.is_empty());
}

#[test]
fn recall_vector_chunks_top_k_zero_does_not_embed_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let chunks = store
        .recall_vector_chunks("anything", &FailingEmbeddingClient, 0)
        .expect("top_k=0 should short-circuit before embedding");

    assert!(chunks.is_empty());
}

#[test]
fn add_vector_chunk_rejects_empty_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("PaymentGateway", "depends_on", "StripeSDK", "test")
        .unwrap();

    let err = store
        .add_vector_chunk(claim_id, " ", "nomic-embed-text")
        .expect_err("blank vector chunk text should be rejected");

    assert!(err.to_string().contains("vector chunk text"));
}

#[test]
fn add_vector_chunk_rejects_empty_embedding_model() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("PaymentGateway", "depends_on", "StripeSDK", "test")
        .unwrap();

    let err = store
        .add_vector_chunk(claim_id, "PaymentGateway depends_on StripeSDK", " ")
        .expect_err("blank embedding model should be rejected");

    assert!(err.to_string().contains("embedding model"));
}

#[test]
fn add_vector_chunk_with_embedding_rejects_empty_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("PaymentGateway", "depends_on", "StripeSDK", "test")
        .unwrap();

    let err = store
        .add_vector_chunk_with_embedding(claim_id, " ", "nomic-embed-text", &[0.1, 0.2])
        .expect_err("blank embedded vector chunk text should be rejected");

    assert!(err.to_string().contains("vector chunk text"));
}

#[test]
fn add_vector_chunk_with_embedding_rejects_empty_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim("PaymentGateway", "depends_on", "StripeSDK", "test")
        .unwrap();

    let err = store
        .add_vector_chunk_with_embedding(
            claim_id,
            "PaymentGateway depends_on StripeSDK",
            "nomic-embed-text",
            &[],
        )
        .expect_err("empty embeddings should be rejected");

    assert!(err.to_string().contains("embedding"));
}
