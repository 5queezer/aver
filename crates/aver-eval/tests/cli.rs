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
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["fixture_name"], "aggregate");
    assert_eq!(json["fixture_count"], 13);
    assert!(json["unsupported_claim_rate"].as_f64().unwrap() >= 0.0);
}
