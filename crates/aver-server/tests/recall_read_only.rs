use aver_server::tools::{AverTools, RecallParams, RememberClaimParams};

#[test]
fn recall_leaves_log_and_database_bytes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let claim = tools
        .remember_claim(RememberClaimParams {
            provenance: None,
            object_kind: None,
            event_id: None,
            strict_atomic: None,
            subject: "synthetic-service".to_string(),
            predicate: "uses".to_string(),
            object: "synthetic-dependency".to_string(),
            confidence: None,
            source: Some("test".to_string()),
            scope: None,
            agent_id: None,
            agent_kind: None,
            context: None,
            valid_from: None,
            valid_until: None,
        })
        .unwrap();
    let log_path = dir.path().join("log.jsonl");
    let db_path = dir.path().join("db.sqlite");
    let log_before = std::fs::read(&log_path).unwrap();
    let db_before = std::fs::read(&db_path).unwrap();

    let recalled = tools
        .recall(RecallParams {
            query: "synthetic-dependency".to_string(),
            hops: None,
            top_k: None,
            scope: None,
            scope_walk: None,
            agent_id: None,
            agent_kind: None,
            predicate: None,
            predicate_walk: None,
            min_confidence: None,
            status: None,
            alpha: None,
            context: None,
        })
        .unwrap();

    assert_eq!(recalled.triples[0].id, claim.id);
    assert_eq!(std::fs::read(log_path).unwrap(), log_before);
    assert_eq!(std::fs::read(db_path).unwrap(), db_before);
}
