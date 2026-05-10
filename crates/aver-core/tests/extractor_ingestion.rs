use aver_core::{AgentKind, ExtractorFact, PrivacyRejection, Provenance, Store};

#[test]
fn deterministic_extractor_facts_become_recallable_extracted_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let ids = store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L10-L20",
            &[ExtractorFact::new(
                "src/lib.rs",
                "defines",
                "Function:ingest",
            )],
        )
        .unwrap();

    assert_eq!(ids.len(), 1);
    let recalled = store.recall_text("ingest").unwrap();
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, ids[0]);
    assert_eq!(recalled[0].provenance, Provenance::Extracted);
    assert_eq!(recalled[0].agent_kind, AgentKind::DeterministicParser);
    assert_eq!(
        recalled[0].source_refs,
        vec!["src/lib.rs#L10-L20".to_string()]
    );
}

#[test]
fn llm_prose_facts_default_to_inferred_unless_explicitly_marked() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let inferred = store
        .ingest_extractor_facts(
            "prose_llm",
            AgentKind::Llm,
            "doc/readme.md#L1-L4",
            &[ExtractorFact::new("Aver", "concerns", "durable_memory")],
        )
        .unwrap()[0];
    let explicit = store
        .ingest_extractor_facts(
            "prose_llm",
            AgentKind::Llm,
            "doc/adr.md#L2-L3",
            &[ExtractorFact::new("Aver", "concerns", "append_first")
                .with_provenance(Provenance::Extracted)],
        )
        .unwrap()[0];

    assert_eq!(
        store.get_claim(inferred).unwrap().provenance,
        Provenance::Inferred
    );
    assert_eq!(
        store.get_claim(explicit).unwrap().provenance,
        Provenance::Extracted
    );
}

#[test]
fn duplicate_extractor_facts_merge_source_refs_without_active_claim_explosion() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let first = store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L10",
            &[ExtractorFact::new(
                "src/lib.rs",
                "defines",
                "Function:ingest",
            )],
        )
        .unwrap()[0];
    store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L11",
            &[ExtractorFact::new(
                "src/lib.rs",
                "defines",
                "Function:ingest",
            )],
        )
        .unwrap();

    let recalled = store.recall_text("Function ingest").unwrap();
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, first);
    assert_eq!(
        store.get_claim(first).unwrap().source_refs,
        vec!["src/lib.rs#L10".to_string(), "src/lib.rs#L11".to_string()]
    );
}

#[test]
fn private_extractor_fact_is_rejected_without_claim_or_log_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L10",
            &[ExtractorFact::new(
                "src/lib.rs",
                "defines",
                "sk-abcdefghijklmnopqrstuvwxyz123456",
            )],
        )
        .unwrap_err();

    assert!(matches!(
        err,
        aver_core::Error::Privacy(PrivacyRejection::OpenAiKey)
    ));
    assert!(
        store
            .recall_text("abcdefghijklmnopqrstuvwxyz123456")
            .unwrap()
            .is_empty()
    );
    assert!(!dir.path().join("log.jsonl").exists());
}

#[test]
fn invalid_fact_in_batch_rejects_without_partial_claim_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L10",
            &[
                ExtractorFact::new("src/lib.rs", "defines", "Function:valid"),
                ExtractorFact::new("src/lib.rs", "defines", " "),
            ],
        )
        .unwrap_err();

    assert!(matches!(err, aver_core::Error::InvalidClaimField { .. }));
    assert!(store.recall_text("valid").unwrap().is_empty());
    assert!(!dir.path().join("log.jsonl").exists());
}

#[test]
fn unknown_extractor_predicate_is_rejected_without_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .ingest_extractor_facts(
            "tree_sitter_rust",
            AgentKind::DeterministicParser,
            "src/lib.rs#L10",
            &[ExtractorFact::new(
                "src/lib.rs",
                "invented_predicate",
                "Function:ingest",
            )],
        )
        .unwrap_err();

    assert!(matches!(err, aver_core::Error::UnknownPredicate { .. }));
    assert!(store.recall_text("ingest").unwrap().is_empty());
    assert!(!dir.path().join("log.jsonl").exists());
}
