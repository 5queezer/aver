use aver_core::{ClaimStatus, Store};
use aver_server::tools::{AverTools, RememberClaimParams};
use serde_json::json;

#[test]
fn mcp_writes_expose_provenance_kind_event_and_atomicity_hint() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let params = serde_json::from_value::<RememberClaimParams>(json!({"subject":"sample","predicate":"concerns","object":"An invented reasoning sentence.","confidence":0.99})).unwrap();
    let result = serde_json::to_value(tools.remember_claim(params).unwrap()).unwrap();
    assert_eq!(result["provenance"], "INFERRED");
    assert_eq!(result["confidence"], 0.75);
    assert_eq!(result["object_kind"], "literal");
    assert!(result["hint"].as_str().unwrap().contains("record_event"));
    let before = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    let strict = serde_json::from_value(json!({"subject":"sample","predicate":"concerns","object":"An invented reasoning sentence.","strict_atomic":true})).unwrap();
    let error = tools.remember_claim(strict).unwrap_err();
    assert!(tools.describe_error(&error).contains("atomic"));
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), before);

    let extracted = serde_json::from_value(json!({"subject":"sample","predicate":"concerns","object":"source-value","provenance":"EXTRACTED","source":"synthetic report"})).unwrap();
    let result = serde_json::to_value(tools.remember_claim(extracted).unwrap()).unwrap();
    assert_eq!(result["provenance"], "EXTRACTED");
    assert_eq!(result["confidence"], 0.9);
    assert!(result.get("hint").is_none());
}

#[test]
fn mcp_default_inference_and_explicit_provenance_reject_bad_inputs_before_append() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let params = serde_json::from_value(
        json!({"subject":"sample","predicate":"concerns","object":"widget"}),
    )
    .unwrap();
    let result = serde_json::to_value(tools.remember_claim(params).unwrap()).unwrap();
    assert_eq!(result["provenance"], "INFERRED");
    assert_eq!(result["confidence"], 0.45);
    let before = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    for extra in [
        json!({"provenance":"GUESS"}),
        json!({"object_kind":"bad"}),
        json!({"event_id":1}),
        json!({"confidence":1.1}),
    ] {
        let mut value = json!({"subject":"sample","predicate":"concerns","object":"widget"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(
            tools
                .remember_claim(serde_json::from_value(value).unwrap())
                .is_err()
        );
    }
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), before);
    assert_eq!(
        Store::open(dir.path())
            .unwrap()
            .get_claim(result["id"].as_i64().unwrap())
            .unwrap()
            .status,
        ClaimStatus::Active
    );
}

#[test]
fn event_then_claim_is_a_cheap_scope_correct_public_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let event = tools.record_event(serde_json::from_value(json!({"session_id":"invented","kind":"decision","payload":"Invented rationale.","source":"synthetic","scope":"proj/sample"})).unwrap()).unwrap();
    let claim = tools.remember_claim(serde_json::from_value(json!({"subject":"decision:invented","predicate":"concerns","object":"topic:sample","event_id":event.id,"scope":"proj/sample"})).unwrap()).unwrap();
    let value = serde_json::to_value(claim).unwrap();
    assert_eq!(value["event_id"], event.id);
    assert_eq!(value["provenance"], "INFERRED");
    let triple=tools.add_triple(serde_json::from_value(json!({"subject":"decision:invented","predicate":"has","object":"-0.18","source":"synthetic result","provenance":"EXTRACTED","event_id":event.id,"scope":"proj/sample"})).unwrap()).unwrap();
    assert_eq!(triple.object_kind, "literal");
    assert_eq!(triple.provenance, "EXTRACTED");
    assert_eq!(triple.event_id, Some(event.id));
    for provenance in ["AMBIGUOUS", "USER_ASSERTED"] {
        let claim=tools.remember_claim(serde_json::from_value(json!({"subject":"sample","predicate":"concerns","object":"widget","source":"synthetic","provenance":provenance})).unwrap()).unwrap();
        assert_eq!(claim.provenance, provenance);
        assert_eq!(
            claim.confidence,
            if provenance == "AMBIGUOUS" {
                0.20
            } else {
                0.95
            }
        );
    }
}
