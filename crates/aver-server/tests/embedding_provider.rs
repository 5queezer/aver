//! Embedding provider selection from configuration, and what a failing
//! OpenAI-compatible provider does to recall and chunk writes.

use std::collections::HashMap;
use std::time::Duration;

use aver_core::vector::OpenAiEmbeddingClient;
use aver_server::tools::{
    AddTripleParams, AddVectorChunkParams, AverTools, RecallParams, embedding_from_vars,
};

fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn unset_configuration_is_unchanged() {
    let (embedder, model) = embedding_from_vars(vars(&[]));
    assert!(embedder.is_none());
    assert_eq!(model, "nomic-embed-text");

    // An OpenAI key in the environment alone opts into nothing.
    let (embedder, model) = embedding_from_vars(vars(&[("OPENAI_API_KEY", "sk-x")]));
    assert!(embedder.is_none());
    assert_eq!(model, "nomic-embed-text");

    let (embedder, model) =
        embedding_from_vars(vars(&[("AVER_EMBEDDING_URL", "http://localhost:11434")]));
    assert!(embedder.is_some());
    assert_eq!(model, "nomic-embed-text");
}

#[test]
fn openai_provider_needs_its_key_and_labels_chunks_with_its_model() {
    let (embedder, model) = embedding_from_vars(vars(&[
        ("AVER_EMBEDDING_PROVIDER", "openai"),
        ("OPENAI_API_KEY", "sk-x"),
    ]));
    assert!(embedder.is_some());
    assert_eq!(model, "text-embedding-3-small");

    // The key variable name and the model are configurable.
    let (embedder, model) = embedding_from_vars(vars(&[
        ("AVER_EMBEDDING_PROVIDER", "openai"),
        ("AVER_EMBEDDING_API_KEY_ENV", "MY_KEY"),
        ("MY_KEY", "sk-x"),
        ("AVER_EMBEDDING_MODEL", "text-embedding-3-large"),
        ("AVER_EMBEDDING_URL", "https://example.invalid/v1"),
    ]));
    assert!(embedder.is_some());
    assert_eq!(model, "text-embedding-3-large");

    // No key, or an unknown provider: keyword recall, no client.
    let (embedder, model) = embedding_from_vars(vars(&[("AVER_EMBEDDING_PROVIDER", "openai")]));
    assert!(embedder.is_none());
    assert_eq!(model, "text-embedding-3-small");
    let (embedder, _) = embedding_from_vars(vars(&[
        ("AVER_EMBEDDING_PROVIDER", "cohere"),
        ("AVER_EMBEDDING_URL", "http://localhost:11434"),
    ]));
    assert!(embedder.is_none());
}

/// A provider that cannot be reached: the chunk is stored text-only, recall
/// answers by keyword, and nothing embeds the chunk later.
#[test]
fn failing_openai_provider_falls_back_to_keyword_and_text_only_chunks() {
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", listener.local_addr().unwrap())
    };
    let dir = tempfile::tempdir().unwrap();
    let client = OpenAiEmbeddingClient::new(closed_port, "text-embedding-3-small", 768, "sk-x")
        .with_timeout(Duration::from_secs(2));
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(Some(Box::new(client)));
    let claim_id = tools
        .add_triple(AddTripleParams {
            subject: "api-gateway".to_string(),
            predicate: "uses".to_string(),
            object: "envoy".to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: None,
            agent_id: None,
            agent_kind: None,
        })
        .unwrap()
        .triple_id;

    tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id,
            text: "api-gateway uses envoy".to_string(),
        })
        .unwrap();
    let recalled = tools
        .recall(RecallParams {
            query: "api-gateway".to_string(),
            hops: None,
            top_k: Some(5),
            scope: None,
            scope_walk: None,
            agent_id: None,
            agent_kind: None,
            predicate: None,
            predicate_walk: None,
            min_confidence: None,
            status: None,
            alpha: None,
        })
        .unwrap();

    assert_eq!(recalled.retrieval, "keyword");
    assert_eq!(recalled.triples[0].id, claim_id);
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    assert!(log.contains("\"kind\":\"add_vector_chunk\""));
    assert!(!log.contains("\"embedding\":["), "chunk must be text-only");
    assert!(!log.contains("sk-x"));
}
