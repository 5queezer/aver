use aver_core::{Store, vector::MockEmbeddingClient};

fn open_store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (store, dir)
}

#[test]
fn embedding_status_returns_zeros_on_empty_store() {
    let (store, _dir) = open_store();
    let (indexed, total) = store.vector_chunk_embedding_status().unwrap();
    assert_eq!(indexed, 0);
    assert_eq!(total, 0);
}

#[test]
fn embedding_status_shows_chunk_without_embedding() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    store
        .add_vector_chunk(claim_id, "Aver uses SQLite", "nomic-embed-text")
        .unwrap();

    let (indexed, total) = store.vector_chunk_embedding_status().unwrap();
    assert_eq!(total, 1);
    assert_eq!(indexed, 0);
}

#[test]
fn backfill_fills_embeddings_with_mock_client() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    store
        .add_vector_chunk(claim_id, "Aver uses SQLite", "nomic-embed-text")
        .unwrap();

    let client = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);
    let filled = store.backfill_vector_embeddings(&client).unwrap();
    assert_eq!(filled, 1);

    let (indexed, total) = store.vector_chunk_embedding_status().unwrap();
    assert_eq!(indexed, 1);
    assert_eq!(total, 1);
}

#[test]
fn backfill_skips_already_embedded_chunks() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);
    store
        .add_embedded_vector_chunk_for_claim(claim_id, "nomic-embed-text", &client)
        .unwrap();

    let filled = store.backfill_vector_embeddings(&client).unwrap();
    assert_eq!(filled, 0);
}

#[test]
fn recall_text_with_embedding_returns_matching_claim() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "prefers", "Rust", "test").unwrap();
    let client = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);
    store
        .add_embedded_vector_chunk_for_claim(claim_id, "nomic-embed-text", &client)
        .unwrap();

    let results = store
        .recall_text_with_embedding("Rust language", &client)
        .unwrap();
    assert!(
        results.iter().any(|c| c.id == claim_id),
        "expected claim {claim_id} in results"
    );
}
