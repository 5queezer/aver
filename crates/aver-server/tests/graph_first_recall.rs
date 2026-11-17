//! ADR-0033 through the server tools: `recall` with `mode: "graph_first"`
//! returns the selected claims with evidence ids, budget and latency, and
//! falls back to the default path when the query names no entity. Without
//! `mode` recall is unchanged.

use aver_server::tools::AverTools;
use serde_json::{Value, json};

fn params<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

fn recall(tools: &AverTools, query: Value) -> Value {
    serde_json::to_value(tools.recall(params(query)).unwrap()).unwrap()
}

fn ids(value: &Value) -> Vec<i64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect()
}

#[test]
fn graph_first_returns_selected_claims_with_evidence_budget_and_latency() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let event = tools
        .record_event(params(json!({
            "session_id": "s1", "kind": "decision", "payload": "picked Stripe for cards",
        })))
        .unwrap();
    let dep = tools
        .add_triple(params(json!({
            "subject": "PaymentGateway", "predicate": "depends_on", "object": "StripeSDK",
            "source": "test", "event_id": event.id,
        })))
        .unwrap();
    tools
        .add_triple(params(json!({
            "subject": "SearchIndex", "predicate": "depends_on", "object": "Tantivy",
            "source": "test",
        })))
        .unwrap();

    let view = recall(
        &tools,
        json!({"query": "what does PaymentGateway depend on", "mode": "graph_first"}),
    );

    assert_eq!(view["retrieval"], "graph_first", "{view}");
    assert_eq!(view["triples"].as_array().unwrap().len(), 1, "{view}");
    assert_eq!(view["triples"][0]["id"], dep.triple_id);
    let gf = &view["graph_first"];
    assert_eq!(gf["route"], "single");
    assert_eq!(gf["anchors"], json!(["PaymentGateway"]));
    assert_eq!(gf["facts"][0]["claim_id"], dep.triple_id);
    assert_eq!(gf["facts"][0]["anchor"], "PaymentGateway");
    assert_eq!(gf["facts"][0]["hop"], 1);
    assert_eq!(ids(&gf["evidence_ids"]["claims"]), vec![dep.triple_id]);
    assert_eq!(ids(&gf["evidence_ids"]["events"]), vec![event.id]);
    assert_eq!(gf["events"][0]["payload"], "picked Stripe for cards");
    assert_eq!(gf["budget"]["max_anchors"], 15);
    assert_eq!(gf["budget"]["max_edges"], 64);
    assert_eq!(gf["budget"]["anchors"], 1);
    assert_eq!(gf["budget"]["selected"], 1);
    assert!(gf["latency_ms"].as_f64().is_some(), "{gf}");
    assert!(gf.get("fallback").is_none(), "{gf}");
}

#[test]
fn recall_without_mode_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    tools
        .add_triple(params(json!({
            "subject": "PaymentGateway", "predicate": "depends_on", "object": "StripeSDK",
            "source": "test",
        })))
        .unwrap();

    let default = recall(&tools, json!({"query": "PaymentGateway"}));
    let hybrid = recall(&tools, json!({"query": "PaymentGateway", "mode": "hybrid"}));

    assert_eq!(default["retrieval"], "keyword");
    assert!(default.get("graph_first").is_none(), "{default}");
    assert_eq!(default, hybrid);
}

#[test]
fn graph_first_falls_back_to_the_default_path_when_no_entity_matches() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let note = tools
        .add_triple(params(json!({
            "subject": "Billing", "predicate": "concerns", "object": "nightly invoice run",
            "source": "test",
        })))
        .unwrap();

    let view = recall(&tools, json!({"query": "invoice", "mode": "graph_first"}));

    assert_eq!(view["retrieval"], "keyword", "{view}");
    assert_eq!(view["triples"][0]["id"], note.triple_id);
    assert_eq!(view["graph_first"]["fallback"], "no_anchor");
    assert!(view["graph_first"]["latency_ms"].as_f64().is_some());
}

#[test]
fn recall_rejects_an_unknown_mode() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .recall(params(json!({"query": "anything", "mode": "graph"})))
        .unwrap_err();

    assert!(err.to_string().contains("invalid mode"), "{err}");
}

#[test]
fn graph_first_never_returns_claims_or_events_from_a_foreign_scope() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let event_b = tools
        .record_event(params(json!({
            "session_id": "s", "kind": "decision", "payload": "b only", "scope": "proj/b",
        })))
        .unwrap();
    let a = tools
        .add_triple(params(json!({
            "subject": "PaymentGateway", "predicate": "depends_on", "object": "StripeSDK",
            "source": "test", "scope": "proj/a",
        })))
        .unwrap();
    tools
        .add_triple(params(json!({
            "subject": "PaymentGateway", "predicate": "depends_on", "object": "AdyenSDK",
            "source": "test", "scope": "proj/b", "event_id": event_b.id,
        })))
        .unwrap();
    tools
        .add_triple(params(json!({
            "subject": "SecretVault", "predicate": "depends_on", "object": "AdyenSDK",
            "source": "test", "scope": "proj/b",
        })))
        .unwrap();
    let scoped = |query: &str| {
        recall(
            &tools,
            json!({"query": query, "mode": "graph_first", "scope": "proj/a", "scope_walk": "exact"}),
        )
    };

    let view = scoped("what does PaymentGateway depend on");
    let foreign = scoped("what does SecretVault depend on");

    assert_eq!(view["retrieval"], "graph_first");
    let triple_ids: Vec<i64> = view["triples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(triple_ids, vec![a.triple_id]);
    for edge in view["subgraph"]["edges"].as_array().unwrap() {
        assert_eq!(edge["scope"], "proj/a", "{view}");
    }
    assert_eq!(view["graph_first"]["events"], json!([]));
    // SecretVault exists only in proj/b, so graph-first has no anchor and
    // the scoped default path answers: whatever it returns is from proj/a.
    assert_eq!(foreign["graph_first"]["fallback"], "no_anchor", "{foreign}");
    for claim in foreign["triples"]
        .as_array()
        .unwrap()
        .iter()
        .chain(foreign["subgraph"]["edges"].as_array().unwrap())
    {
        assert_eq!(claim["scope"], "proj/a", "{foreign}");
    }
}
