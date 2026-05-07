//! T93 — v0.6 starts with offline parsing for structured prose extraction output.

use memory_extractor::{ExtractedFact, parse_prose_facts};

#[test]
fn parse_prose_facts_reads_structured_llm_output() {
    let facts = parse_prose_facts(
        r#"{"facts":[{"subject":"User","predicate":"prefers","object":"Rust"}]}"#,
    )
    .unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "User".to_string(),
            predicate: "prefers".to_string(),
            object: "Rust".to_string(),
        }]
    );
}

#[test]
fn parse_prose_facts_rejects_empty_subject() {
    let result =
        parse_prose_facts(r#"{"facts":[{"subject":"","predicate":"prefers","object":"Rust"}]}"#);

    assert!(result.is_err());
}
