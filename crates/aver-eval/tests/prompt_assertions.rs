use aver_eval::prompt_assertions::{
    BeamPromptVersion, PromptContractError, PromptContractSpec, PromptKind, assert_prompt_contract,
    contract_for,
};

#[test]
fn prompt_contract_passes_when_all_deterministic_requirements_hold() {
    let prompt = "SYSTEM:\nUse only the provided context.\n\nQUESTION:\nWhat changed?\n\nANSWER:";
    let contract = PromptContractSpec::new("test_prompt_v1")
        .require_text("Use only the provided context")
        .forbid_text("ground-truth-only-answer")
        .require_section("QUESTION:")
        .max_chars(200);

    assert_prompt_contract(prompt, &contract).unwrap();
}

#[test]
fn prompt_contract_reports_all_missing_required_text() {
    let contract = PromptContractSpec::new("test_prompt_v1")
        .require_text("Use only the provided context")
        .require_text("Return only JSON");

    let error = assert_prompt_contract("ANSWER:", &contract).unwrap_err();

    assert_eq!(
        error,
        PromptContractError {
            contract_id: "test_prompt_v1".to_string(),
            failures: vec![
                "missing required text: Use only the provided context".to_string(),
                "missing required text: Return only JSON".to_string(),
            ],
        }
    );
}

#[test]
fn prompt_contract_rejects_forbidden_text_and_unresolved_placeholders() {
    let contract = PromptContractSpec::new("test_prompt_v1")
        .forbid_text("ground-truth-only-answer")
        .reject_unresolved_placeholders(true);

    let error = assert_prompt_contract(
        "QUESTION: {{question}}\nANSWER: ground-truth-only-answer",
        &contract,
    )
    .unwrap_err();

    assert_eq!(
        error.failures,
        vec![
            "contains forbidden text: ground-truth-only-answer".to_string(),
            "contains unresolved template placeholder".to_string(),
        ]
    );
}

#[test]
fn prompt_contract_enforces_required_sections_and_length() {
    let contract = PromptContractSpec::new("test_prompt_v1")
        .require_section("QUESTION:")
        .require_section("ANSWER:")
        .max_chars(10);

    let error = assert_prompt_contract("QUESTION: this is too long", &contract).unwrap_err();

    assert_eq!(
        error.failures,
        vec![
            "missing required section: ANSWER:".to_string(),
            "prompt length 26 exceeds max 10 chars".to_string(),
        ]
    );
}

#[test]
fn beam_answer_v1_contract_accepts_current_answer_prompt() {
    let prompt = aver_eval::beam::answer_prompt(
        "When is launch?",
        "ground-truth-only-answer",
        &["[2024-03-15] Launch is April 1".to_string()],
    );
    let contract = contract_for(PromptKind::BeamAnswer(BeamPromptVersion::V1));

    assert_prompt_contract(&prompt, &contract).unwrap();
}

#[test]
fn beam_judge_v1_contract_accepts_current_judge_prompt() {
    let prompt =
        aver_eval::beam::judge_prompt("What changed?", "It changed today.", "Mentions today");
    let contract = contract_for(PromptKind::BeamJudge(BeamPromptVersion::V1));

    assert_prompt_contract(&prompt, &contract).unwrap();
}
