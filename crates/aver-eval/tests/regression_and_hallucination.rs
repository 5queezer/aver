use aver_eval::{
    HallucinationCaseResult, HallucinationRunReport, QuerySuiteRegressionReport,
    QuerySuiteThresholds,
};

#[test]
fn query_suite_regression_fails_when_metric_drops_more_than_five_percent() {
    let report = QuerySuiteRegressionReport::compare(
        QuerySuiteThresholds::default(),
        0.90,
        0.80,
        0.70,
        0.70,
        0.50,
        0.50,
    );

    assert!(report.failed);
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.metric == "recall"));
    assert!(report
        .failures
        .iter()
        .all(|failure| failure.drop_fraction > 0.05));
}

#[test]
fn hallucination_report_computes_memory_on_boundary_metrics() {
    let report = HallucinationRunReport::from_cases(vec![
        HallucinationCaseResult {
            id: "kept".to_string(),
            memory_off_accurate: false,
            memory_on_accurate: true,
            memory_off_hallucinated: true,
            memory_on_hallucinated: false,
        },
        HallucinationCaseResult {
            id: "laundered_bad_memory".to_string(),
            memory_off_accurate: true,
            memory_on_accurate: false,
            memory_off_hallucinated: false,
            memory_on_hallucinated: true,
        },
    ]);

    assert_eq!(report.case_count, 2);
    assert_eq!(report.memory_on_minus_off_accuracy, 0.0);
    assert_eq!(report.memory_on_hallucination_rate, 0.5);
    assert_eq!(report.memory_off_hallucination_rate, 0.5);
    assert_eq!(
        report.memory_on_worse_case_ids,
        vec!["laundered_bad_memory"]
    );
}
