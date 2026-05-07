//! T14 — vector chunk metadata can be written for a claim before sqlite-vss
//! indexing is wired in.

use memory_core::Store;

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
