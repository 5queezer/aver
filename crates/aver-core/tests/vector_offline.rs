use aver_core::{
    Store,
    vector::{EmbeddingClient, EmbeddingError, MockEmbeddingClient},
};

struct FailsFirstEmbeddingClient(std::sync::atomic::AtomicUsize);

impl EmbeddingClient for FailsFirstEmbeddingClient {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Err(EmbeddingError::Http("transient".to_string()))
        } else {
            Ok(vec![1.0, 0.0, 0.0])
        }
    }
}

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
fn backfill_processes_at_most_the_requested_chunk_limit() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    for text in ["chunk one", "chunk two", "chunk three"] {
        store
            .add_vector_chunk(claim_id, text, "nomic-embed-text")
            .unwrap();
    }
    let client = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);

    let filled = store
        .backfill_vector_embeddings_with_limit(&client, 2)
        .unwrap();

    assert_eq!(filled, 2);
    assert_eq!(store.vector_chunk_embedding_status().unwrap(), (2, 3));
}

#[test]
fn backfill_keeps_progress_when_one_embedding_fails() {
    let (store, _dir) = open_store();
    let claim_id = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    for text in ["fails first", "succeeds second"] {
        store
            .add_vector_chunk(claim_id, text, "nomic-embed-text")
            .unwrap();
    }
    let client = FailsFirstEmbeddingClient(std::sync::atomic::AtomicUsize::new(0));

    let filled = store
        .backfill_vector_embeddings_with_limit(&client, 2)
        .unwrap();

    assert_eq!(filled, 1);
    assert_eq!(store.vector_chunk_embedding_status().unwrap(), (1, 2));
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
