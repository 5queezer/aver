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
