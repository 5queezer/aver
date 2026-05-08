//! T9/T10/T11 — v0.2 starts with deterministic Ollama embeddings JSON shapes
//! before any network I/O. ADR-0013 requires Rust HTTP to local Ollama; ADR-0004
//! supplies HybridRAG's vector side.

use aver_core::vector::{
    EmbeddingClient, MockEmbeddingClient, OllamaEmbeddingClient, OllamaEmbeddingRequest,
    OllamaEmbeddingResponse, VectorBackend, VectorIndexConfig, cosine_similarity,
    normalized_cosine_score,
};

#[test]
fn ollama_embedding_request_serializes_model_and_prompt() {
    let request = OllamaEmbeddingRequest::new("nomic-embed-text", "auth service depends on stripe");

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["model"], "nomic-embed-text");
    assert_eq!(json["prompt"], "auth service depends on stripe");
}

#[test]
fn ollama_embedding_response_deserializes_embedding_vector() {
    let response: OllamaEmbeddingResponse =
        serde_json::from_str(r#"{"embedding":[0.125,-0.5,1.0]}"#).unwrap();

    assert_eq!(response.embedding, vec![0.125, -0.5, 1.0]);
}

#[test]
fn ollama_embedding_client_normalizes_embeddings_endpoint_url() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434/", "nomic-embed-text");

    assert_eq!(
        client.embeddings_url(),
        "http://localhost:11434/api/embeddings"
    );
}

#[test]
fn ollama_embedding_client_builds_request_with_configured_model() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434", "nomic-embed-text");

    let json = serde_json::to_value(client.request("remembered claim text")).unwrap();

    assert_eq!(json["model"], "nomic-embed-text");
    assert_eq!(json["prompt"], "remembered claim text");
}

#[test]
fn vector_backend_defaults_to_sqlite_vss() {
    assert_eq!(VectorBackend::default(), VectorBackend::SqliteVss);
}

#[test]
fn vector_backend_parses_qdrant_opt_in() {
    assert_eq!(
        "qdrant".parse::<VectorBackend>().unwrap(),
        VectorBackend::Qdrant
    );
}

#[test]
fn vector_backend_displays_config_values() {
    assert_eq!(VectorBackend::SqliteVss.to_string(), "sqlite-vss");
    assert_eq!(VectorBackend::Qdrant.to_string(), "qdrant");
}

#[test]
fn vector_backend_from_optional_config_defaults_when_unset() {
    assert_eq!(
        VectorBackend::from_optional_config(None).unwrap(),
        VectorBackend::SqliteVss
    );
}

#[test]
fn vector_backend_config_parser_trims_whitespace() {
    assert_eq!(
        VectorBackend::from_optional_config(Some(" qdrant ")).unwrap(),
        VectorBackend::Qdrant
    );
}

#[test]
fn vector_index_config_defaults_to_sqlite_vss_and_nomic_model() {
    let config = VectorIndexConfig::default();

    assert_eq!(config.backend, VectorBackend::SqliteVss);
    assert_eq!(config.embedding_model, "nomic-embed-text");
}

#[test]
fn vector_index_config_parses_optional_backend_and_model() {
    let config =
        VectorIndexConfig::from_optional_config(Some(" qdrant "), Some("all-minilm")).unwrap();

    assert_eq!(config.backend, VectorBackend::Qdrant);
    assert_eq!(config.embedding_model, "all-minilm");
}

#[test]
fn vector_backend_empty_optional_config_uses_default() {
    assert_eq!(
        VectorBackend::from_optional_config(Some("  ")).unwrap(),
        VectorBackend::SqliteVss
    );
}

#[test]
fn ollama_embedding_client_parses_response_body_to_vector() {
    let client = OllamaEmbeddingClient::new("http://localhost:11434", "nomic-embed-text");

    let embedding = client
        .parse_response_body(r#"{"embedding":[0.25,0.75]}"#)
        .unwrap();

    assert_eq!(embedding, vec![0.25, 0.75]);
}

#[test]
fn cosine_similarity_returns_one_for_identical_vectors() {
    let score = cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap();

    assert!((score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn cosine_similarity_returns_none_for_zero_vector() {
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), None);
}

#[test]
fn normalized_cosine_score_maps_opposite_vectors_to_zero() {
    let score = normalized_cosine_score(&[1.0, 0.0], &[-1.0, 0.0]).unwrap();

    assert!((score - 0.0).abs() < f32::EPSILON);
}

#[test]
fn mock_embedding_client_returns_deterministic_vector() {
    let client = MockEmbeddingClient::new(vec![0.1, 0.2]);

    assert_eq!(client.embed("hello memory").unwrap(), vec![0.1, 0.2]);
}

#[test]
fn embedding_client_trait_allows_offline_vector_tests() {
    let client: &dyn EmbeddingClient = &MockEmbeddingClient::new(vec![0.3, 0.4]);

    assert_eq!(client.embed("no network").unwrap(), vec![0.3, 0.4]);
}

#[test]
fn vector_claim_recall_skips_superseded_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = aver_core::Store::open(dir.path()).unwrap();
    let stale = store
        .add_claim("StripeSDK", "status", "deprecated", "session-1")
        .unwrap();
    let current = store
        .add_claim("StripeSDK", "status", "current", "session-2")
        .unwrap();
    store
        .add_vector_chunk_with_embedding(stale, "StripeSDK status deprecated", "mock", &[1.0, 0.0])
        .unwrap();
    store
        .add_vector_chunk_with_embedding(current, "StripeSDK status current", "mock", &[0.9, 0.1])
        .unwrap();
    store.consolidate().unwrap();

    let recalled = store
        .recall_vector_claims(
            "StripeSDK status",
            &MockEmbeddingClient::new(vec![1.0, 0.0]),
            2,
        )
        .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, current);
}
