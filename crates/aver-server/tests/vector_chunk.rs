use aver_server::tools::{AddTripleParams, AddVectorChunkParams, AverTools};

fn synthetic_openai_key() -> String {
    ["sk", "-", "abcdefghijklmnopqrstuvwxyz1234567890"].concat()
}

#[test]
fn add_vector_chunk_persists_and_is_retrievable() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    // First remember a claim to get a valid claim_id.
    let triple = tools
        .add_triple(AddTripleParams {
            subject: "Aver".to_string(),
            predicate: "uses".to_string(),
            object: "SQLite".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: None,
        })
        .unwrap();

    let chunk = tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id: triple.triple_id,
            text: "Aver uses SQLite for durable storage".to_string(),
        })
        .unwrap();

    assert!(chunk.id > 0);
    assert_eq!(chunk.claim_id, triple.triple_id);
    assert_eq!(chunk.text, "Aver uses SQLite for durable storage");
}

#[test]
fn add_vector_chunk_rejects_empty_text() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let triple = tools
        .add_triple(AddTripleParams {
            subject: "Aver".to_string(),
            predicate: "uses".to_string(),
            object: "SQLite".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: None,
        })
        .unwrap();

    let err = tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id: triple.triple_id,
            text: "  ".to_string(),
        })
        .unwrap_err();

    assert!(err.to_string().contains("text"));
}

#[test]
fn add_vector_chunk_rejects_secret_text() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let triple = tools
        .add_triple(AddTripleParams {
            subject: "Aver".to_string(),
            predicate: "uses".to_string(),
            object: "SQLite".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: None,
        })
        .unwrap();

    let err = tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id: triple.triple_id,
            text: format!("do not persist {}", synthetic_openai_key()),
        })
        .unwrap_err();

    assert!(
        err.to_string().contains("privacy filter"),
        "unexpected error: {err}"
    );
}

#[test]
fn add_vector_chunk_rejects_nonexistent_claim() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id: 9999,
            text: "some text".to_string(),
        })
        .unwrap_err();

    assert!(
        err.to_string().contains("claim") || err.to_string().contains("missing"),
        "unexpected error: {err}"
    );
}
