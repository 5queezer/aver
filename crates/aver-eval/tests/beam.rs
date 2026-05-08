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
fn beam_redacts_secret_like_tokens_before_ingestion() {
    let raw = "Configure API_KEY=q7Zp9Lm2Kx8Vn4Rb6Ty0Wc3Ae5Gu for local testing";

    let sanitized = aver_eval::beam::sanitize_memory_text(raw);

    assert_eq!(
        sanitized,
        "Configure API_KEY=[REDACTED_SECRET] for local testing"
    );
    assert!(aver_core::privacy_filter(&sanitized).is_ok());
}

#[test]
fn beam_answer_prompt_instructs_direct_grounded_answers() {
    let prompt = aver_eval::beam::answer_prompt("When is launch?", "April 1", &[]);

    assert!(prompt.contains("Answer the question directly"));
    assert!(prompt.contains("Do not respond with generic meta-commentary"));
    assert!(prompt.contains("If retrieved memories conflict"));
}

#[test]
fn openai_embedding_request_serializes_model_and_input() {
    let request =
        aver_eval::beam::OpenAiEmbeddingRequest::new("text-embedding-3-small", "hello memory");

    let json = serde_json::to_value(request).unwrap();

    assert_eq!(json["model"], "text-embedding-3-small");
    assert_eq!(json["input"], "hello memory");
}

#[test]
fn openai_chat_request_uses_json_object_format_for_judge_mode() {
    let request = aver_eval::beam::OpenAiChatRequest::new("gpt-4o-mini", "judge", true);

    let json = serde_json::to_value(request).unwrap();

    assert_eq!(json["model"], "gpt-4o-mini");
    assert_eq!(json["messages"][0]["role"], "user");
    assert_eq!(json["messages"][0]["content"], "judge");
    assert_eq!(json["response_format"]["type"], "json_object");
}

#[test]
fn beam_provider_parses_openai() {
    assert_eq!(
        "openai".parse::<aver_eval::beam::BeamProvider>().unwrap(),
        aver_eval::beam::BeamProvider::OpenAi
    );
}

#[test]
fn beam_answer_prompt_contains_question_and_context_without_reference_answer_leakage() {
    let prompt = aver_eval::beam::answer_prompt(
        "What language?",
        "ground-truth-only-answer",
        &["[March-15-2024] Alice likes Rust".to_string()],
    );

    assert!(prompt.contains("What language?"));
    assert!(prompt.contains("Alice likes Rust"));
    assert!(!prompt.contains("ground-truth-only-answer"));
}
