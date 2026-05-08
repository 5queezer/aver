#[test]
fn beam_dataset_deserializes_tiny_fixture() {
    let dataset = aver_eval::beam::load_dataset_str(
        r#"{
          "split":"test",
          "num_conversations":1,
          "total_questions":1,
          "conversations":[{
            "id":"1",
            "category":"personal",
            "title":"Tiny",
            "user_messages":[{"role":"user","content":"Alice likes Rust","time_anchor":"2024-03-15"}],
            "total_turns":1,
            "questions":[{"ability":"knowledge_update","question":"What does Alice like?","reference_answer":"Rust","rubric":["Mentions Rust"]}]
          }]
        }"#,
    )
    .unwrap();

    assert_eq!(
        dataset.conversations[0].questions[0].ability,
        "knowledge_update"
    );
    assert_eq!(
        dataset.conversations[0].questions[0].rubric_items(),
        vec!["Mentions Rust"]
    );
}

#[test]
fn beam_judge_response_parses_and_clamps_scores() {
    assert_eq!(
        aver_eval::beam::parse_judge_score(r#"{"score":1.0,"reason":"ok"}"#),
        1.0
    );
    assert_eq!(aver_eval::beam::parse_judge_score(r#"{"score":0.49}"#), 0.5);
    assert_eq!(aver_eval::beam::parse_judge_score("not json"), 0.0);
}

#[test]
fn beam_answer_prompt_contains_question_reference_and_context() {
    let prompt = aver_eval::beam::answer_prompt(
        "What language?",
        "Rust",
        &["[March-15-2024] Alice likes Rust".to_string()],
    );

    assert!(prompt.contains("What language?"));
    assert!(prompt.contains("Rust"));
    assert!(prompt.contains("Alice likes Rust"));
}
