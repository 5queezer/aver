use std::process::Command;

#[test]
fn aver_eval_binary_runs_fixture_and_prints_metrics_json() {
    let output = Command::new(std::env::var("CARGO_BIN_EXE_aver-eval").unwrap())
        .arg("../../eval/fixtures/basic_recall.json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["fixture_name"], "basic_recall");
    assert!(json["mean_recall_at_k"].is_number());
    assert!(json["query_results"].is_array());
}

#[test]
fn aver_eval_binary_aggregates_multiple_fixtures() {
    let output = Command::new(std::env::var("CARGO_BIN_EXE_aver-eval").unwrap())
        .arg("../../eval/fixtures/conflict_and_noise.json")
        .arg("../../eval/fixtures/ambiguous_single_token.json")
        .arg("../../eval/fixtures/single_token_multi_answer.json")
        .arg("../../eval/fixtures/natural_query_noise.json")
        .arg("../../eval/fixtures/camel_case_memory_terms.json")
        .arg("../../eval/fixtures/abstention.json")
        .arg("../../eval/fixtures/predicate_role_morphology.json")
        .arg("../../eval/fixtures/acronym_expansion.json")
        .arg("../../eval/fixtures/camel_case_acronym_query.json")
        .arg("../../eval/fixtures/versioned_acronym_identifier.json")
        .arg("../../eval/fixtures/versioned_camel_case_identifier.json")
        .arg("../../eval/fixtures/mixed_case_numeric_identifier.json")
        .arg("../../eval/fixtures/mixed_case_prefix_identifier.json")
        .arg("../../eval/fixtures/ies_plural_morphology.json")
        .arg("../../eval/fixtures/multi_digit_version_identifier.json")
        .arg("../../eval/fixtures/v_prefixed_version_query.json")
        .arg("../../eval/fixtures/irregular_plural_morphology.json")
        .arg("../../eval/fixtures/people_plural_morphology.json")
        .arg("../../eval/fixtures/beam_knowledge_update.json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["fixture_name"], "aggregate");
    assert_eq!(json["fixture_count"], 19);
    assert_eq!(json["query_count"], 28);
    assert!(json["unsupported_claim_rate"].as_f64().unwrap() >= 0.0);
}

fn run(args: &[&str]) -> serde_json::Value {
    let output = Command::new(std::env::var("CARGO_BIN_EXE_aver-eval").unwrap())
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("stdout must be one JSON document")
}

#[test]
fn several_graph_first_fixtures_print_one_json_document() {
    let ops = "../../eval/fixtures/graph_first_ops.json";

    let json = run(&[ops, ops]);

    assert_eq!(json["graphfirst"].as_array().unwrap().len(), 2);
    assert!(json.get("keyword").is_none());
}

#[test]
fn mixed_fixtures_print_one_json_document_with_keyword_aggregation() {
    let json = run(&[
        "../../eval/fixtures/graph_first_ops.json",
        "../../eval/fixtures/basic_recall.json",
        "../../eval/fixtures/abstention.json",
    ]);

    assert_eq!(json["graphfirst"].as_array().unwrap().len(), 1);
    assert_eq!(json["keyword"]["fixture_name"], "aggregate");
    assert_eq!(json["keyword"]["fixture_count"], 2);
}

#[test]
fn one_graph_first_fixture_prints_its_report_unwrapped() {
    let json = run(&["../../eval/fixtures/graph_first_ops.json"]);

    assert!(json.get("graphfirst").is_none());
    assert!(json.is_object());
}
