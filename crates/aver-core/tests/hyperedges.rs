use aver_core::{ClaimStatus, Error, HyperedgeInput, HyperedgeParticipantInput, Provenance, Store};

fn sample_hyperedge() -> HyperedgeInput {
    HyperedgeInput {
        predicate: "deployment".to_string(),
        provenance: Provenance::UserAsserted,
        confidence: 0.9,
        source_refs: vec!["test_session".to_string()],
        participants: vec![
            HyperedgeParticipantInput {
                role: "service".to_string(),
                entity: "api".to_string(),
            },
            HyperedgeParticipantInput {
                role: "environment".to_string(),
                entity: "prod".to_string(),
            },
        ],
    }
}

fn participant(role: &str, entity: &str) -> HyperedgeParticipantInput {
    HyperedgeParticipantInput {
        role: role.to_string(),
        entity: entity.to_string(),
    }
}

#[test]
fn hyperedge_predicates_follow_ontology_policy() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .add_hyperedge(HyperedgeInput {
            predicate: "new_user_relation".to_string(),
            provenance: Provenance::UserAsserted,
            confidence: 0.9,
            source_refs: vec!["manual".to_string()],
            participants: vec![participant("left", "A"), participant("right", "B")],
        })
        .expect("user asserted hyperedges may extend ontology");

    let err = store
        .add_hyperedge(HyperedgeInput {
            predicate: "new_extracted_relation".to_string(),
            provenance: Provenance::Extracted,
            confidence: 0.9,
            source_refs: vec!["parser".to_string()],
            participants: vec![participant("left", "C"), participant("right", "D")],
        })
        .expect_err("extracted hyperedges must not extend ontology silently");

    assert!(matches!(
        err,
        Error::UnknownPredicate { name } if name == "new_extracted_relation"
    ));
    assert!(
        store
            .list_active_hyperedges()
            .unwrap()
            .iter()
            .all(|edge| edge.predicate != "new_extracted_relation")
    );
}

#[test]
fn migration_creates_hyperedge_tables_with_expected_columns() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();

    let hyperedge_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(hyperedges)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        hyperedge_columns,
        vec![
            "id",
            "predicate",
            "provenance",
            "confidence",
            "source_refs",
            "status",
            "created_at",
            "updated_at",
        ]
    );

    let participant_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(hyperedge_participants)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        participant_columns,
        vec!["id", "hyperedge_id", "role", "entity"]
    );
}

#[test]
fn add_hyperedge_appends_audit_log_before_sqlite_projection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_hyperedge_projection
         BEFORE INSERT ON hyperedges
         BEGIN
           SELECT RAISE(FAIL, 'projection failed after audit append');
         END;",
    )
    .unwrap();

    let err = store.add_hyperedge(sample_hyperedge()).unwrap_err();
    assert!(err.to_string().contains("projection failed"));

    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(entry["kind"], "add_hyperedge");
    assert_eq!(entry["predicate"], "deployment");
    assert_eq!(entry["participants"][0]["role"], "service");
}

#[test]
fn add_hyperedge_rolls_back_projection_when_participant_insert_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_participant_projection
         BEFORE INSERT ON hyperedge_participants
         WHEN NEW.role = 'environment'
         BEGIN
           SELECT RAISE(FAIL, 'participant projection failed');
         END;",
    )
    .unwrap();

    let err = store.add_hyperedge(sample_hyperedge()).unwrap_err();
    assert!(err.to_string().contains("participant projection failed"));

    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1, "audit log remains append-first");
    let hyperedge_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM hyperedges", [], |row| row.get(0))
        .unwrap();
    let participant_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM hyperedge_participants", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(hyperedge_rows, 0);
    assert_eq!(participant_rows, 0);
}

#[test]
fn add_hyperedge_rejects_private_content_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut input = sample_hyperedge();
    input.participants[0].entity = "sk-abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGH".to_string();

    let err = store.add_hyperedge(input).unwrap_err();
    assert!(err.to_string().contains("privacy filter rejected"));
    assert!(!dir.path().join("log.jsonl").exists());
}

#[test]
fn add_hyperedge_validates_participants() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut input = sample_hyperedge();
    input.participants = vec![HyperedgeParticipantInput {
        role: " ".to_string(),
        entity: "api".to_string(),
    }];

    let err = store.add_hyperedge(input).unwrap_err();
    assert!(
        err.to_string()
            .contains("invalid hyperedge participant role")
    );
}

#[test]
fn list_recall_and_traverse_return_only_active_hyperedges() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let active_id = store.add_hyperedge(sample_hyperedge()).unwrap();
    let mut inactive = sample_hyperedge();
    inactive.predicate = "incident".to_string();
    inactive.participants[0].entity = "api".to_string();
    inactive.participants[1].entity = "staging".to_string();
    let inactive_id = store.add_hyperedge(inactive).unwrap();

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    conn.execute(
        "UPDATE hyperedges SET status = 'SUPERSEDED' WHERE id = ?1",
        [inactive_id],
    )
    .unwrap();

    let listed = store.list_active_hyperedges().unwrap();
    assert_eq!(
        listed.iter().map(|edge| edge.id).collect::<Vec<_>>(),
        vec![active_id]
    );

    let recalled = store.recall_hyperedges("prod").unwrap();
    assert_eq!(
        recalled.iter().map(|edge| edge.id).collect::<Vec<_>>(),
        vec![active_id]
    );

    let traversed = store.traverse_hyperedges("api").unwrap();
    assert_eq!(
        traversed.iter().map(|edge| edge.id).collect::<Vec<_>>(),
        vec![active_id]
    );
    assert_eq!(traversed[0].status, ClaimStatus::Active);
}
