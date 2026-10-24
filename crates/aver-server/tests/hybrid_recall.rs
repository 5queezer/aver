//! ADR-0004 hybrid recall through `AverTools::recall`: real chunks, alpha,
//! filters on hybrid output, and the keyword fallback when embedding fails.

use std::collections::HashMap;

use aver_core::vector::{EmbeddingClient, EmbeddingError, MockEmbeddingClient};
use aver_server::tools::{AddTripleParams, AddVectorChunkParams, AverTools, RecallParams};

/// Maps known texts to fixed vectors; any other text is an embedding error,
/// standing in for an unreachable Ollama.
struct LookupEmbedder(HashMap<&'static str, Vec<f32>>);

impl EmbeddingClient for LookupEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.0
            .get(text)
            .cloned()
            .ok_or_else(|| EmbeddingError::Http("connection refused".to_string()))
    }
}

const QUERY: &str = "billing charges";
const LEDGER_CHUNK: &str = "ledger rows are append only";
const CARD_CHUNK: &str = "card payments settle nightly";

fn lookup_embedder() -> LookupEmbedder {
    LookupEmbedder(HashMap::from([
        (QUERY, vec![1.0, 0.0]),
        (LEDGER_CHUNK, vec![-1.0, 0.0]),
        (CARD_CHUNK, vec![1.0, 0.0]),
    ]))
}

fn recall_params(query: &str, alpha: Option<f64>) -> RecallParams {
    RecallParams {
        query: query.to_string(),
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
        alpha,
    }
}

fn add_claim(tools: &AverTools, subject: &str, object: &str, scope: Option<&str>) -> i64 {
    tools
        .add_triple(AddTripleParams {
            subject: subject.to_string(),
            predicate: "relates_to".to_string(),
            object: object.to_string(),
            confidence: None,
            source: "test".to_string(),
            scope: scope.map(str::to_string),
            agent_id: None,
            agent_kind: None,
        })
        .unwrap()
        .triple_id
}

fn add_chunk(tools: &AverTools, claim_id: i64, text: &str) {
    tools
        .add_vector_chunk(AddVectorChunkParams {
            claim_id,
            text: text.to_string(),
        })
        .unwrap();
}

/// `billing -> ledger` matches the query text (graph score 0.5) but its
/// chunk points away from the query; `invoice-worker -> queue` shares no
/// query token (graph score 0) but its chunk equals the query embedding.
fn seeded_tools(dir: &tempfile::TempDir) -> (AverTools, i64, i64) {
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(Some(Box::new(lookup_embedder())));
    let graph_claim = add_claim(&tools, "billing", "ledger", None);
    let vector_claim = add_claim(&tools, "invoice-worker", "queue", None);
    add_chunk(&tools, graph_claim, LEDGER_CHUNK);
    add_chunk(&tools, vector_claim, CARD_CHUNK);
    (tools, graph_claim, vector_claim)
}

#[test]
fn hybrid_recall_returns_real_chunks_and_reports_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, _, vector_claim) = seeded_tools(&dir);

    let recalled = tools.recall(recall_params(QUERY, Some(0.9))).unwrap();

    assert_eq!(recalled.retrieval, "hybrid");
    assert_eq!(recalled.alpha, Some(0.9));
    // The vector-only claim is reachable at all: keyword recall never finds it.
    assert_eq!(recalled.triples[0].id, vector_claim);
    assert_eq!(recalled.chunks, vec![CARD_CHUNK, LEDGER_CHUNK]);
}

#[test]
fn hybrid_recall_respects_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, graph_claim, vector_claim) = seeded_tools(&dir);

    let vector_heavy = tools.recall(recall_params(QUERY, Some(1.0))).unwrap();
    let graph_heavy = tools.recall(recall_params(QUERY, Some(0.0))).unwrap();

    let ids = |view: &aver_server::tools::RecallView| {
        view.triples
            .iter()
            .map(|claim| claim.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&vector_heavy), vec![vector_claim, graph_claim]);
    assert_eq!(ids(&graph_heavy), vec![graph_claim, vector_claim]);
}

#[test]
fn hybrid_recall_rejects_out_of_range_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, _, _) = seeded_tools(&dir);

    let err = tools.recall(recall_params(QUERY, Some(1.5))).unwrap_err();

    assert!(err.to_string().contains("invalid alpha"), "{err}");
}

#[test]
fn hybrid_recall_applies_scope_filter_to_core_output() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(Some(Box::new(MockEmbeddingClient::new(vec![0.5, 0.5]))));
    let inside = add_claim(&tools, "billing", "ledger", Some("proj/a"));
    let outside = add_claim(&tools, "billing", "archive", Some("proj/b"));
    add_chunk(&tools, inside, "inside chunk");
    add_chunk(&tools, outside, "outside chunk");

    let recalled = tools
        .recall(RecallParams {
            scope: Some("proj/a".to_string()),
            scope_walk: Some("exact".to_string()),
            ..recall_params(QUERY, None)
        })
        .unwrap();

    assert_eq!(recalled.retrieval, "hybrid");
    let ids: Vec<i64> = recalled.triples.iter().map(|claim| claim.id).collect();
    assert_eq!(ids, vec![inside]);
    assert_eq!(recalled.chunks, vec!["inside chunk"]);
}

#[test]
fn recall_falls_back_to_keyword_when_query_embedding_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, graph_claim, _) = seeded_tools(&dir);

    // The lookup embedder knows no "billing" query: it errors like a downed
    // Ollama, and recall must still answer from the keyword path.
    let recalled = tools.recall(recall_params("billing", None)).unwrap();

    assert_eq!(recalled.retrieval, "keyword");
    assert_eq!(recalled.alpha, None);
    assert!(recalled.chunks.is_empty());
    let ids: Vec<i64> = recalled.triples.iter().map(|claim| claim.id).collect();
    assert_eq!(ids, vec![graph_claim]);
}

#[test]
fn chunk_embedding_failure_stores_text_and_recall_stays_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(Some(Box::new(lookup_embedder())));
    let claim = add_claim(&tools, "billing", "ledger", None);
    // Unknown text: embedding fails, the chunk is still stored (text only).
    add_chunk(&tools, claim, "text the embedder cannot handle");

    let recalled = tools.recall(recall_params(QUERY, None)).unwrap();

    assert_eq!(recalled.retrieval, "keyword");
    assert_eq!(recalled.triples[0].id, claim);
}

#[test]
fn recall_without_embedding_client_is_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path())
        .unwrap()
        .with_embedding_client(None);
    let claim = add_claim(&tools, "billing", "ledger", None);
    add_chunk(&tools, claim, LEDGER_CHUNK);

    let recalled = tools.recall(recall_params(QUERY, None)).unwrap();

    assert_eq!(recalled.retrieval, "keyword");
    assert!(recalled.chunks.is_empty());
}

#[test]
fn non_active_status_filter_uses_keyword_path() {
    let dir = tempfile::tempdir().unwrap();
    let (tools, _, _) = seeded_tools(&dir);

    let recalled = tools
        .recall(RecallParams {
            status: Some("any".to_string()),
            ..recall_params(QUERY, None)
        })
        .unwrap();

    assert_eq!(recalled.retrieval, "keyword");
}
