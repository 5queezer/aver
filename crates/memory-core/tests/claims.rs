//! T2 — adding a claim writes to the JSONL log first (source of truth,
//! ADR-0005) and mirrors into SQLite. Both views are queryable.

use memory_core::Store;

#[test]
fn add_claim_appends_to_episodic_log_and_mirrors_to_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim("auth_service", "depends_on", "stripe_sdk", "test_session")
        .expect("add_claim should succeed");

    // SQLite mirror: claim is queryable by id, with the right s/p/o.
    let row = store
        .get_claim(claim_id)
        .expect("get_claim should return the row");
    assert_eq!(row.subject, "auth_service");
    assert_eq!(row.predicate, "depends_on");
    assert_eq!(row.object, "stripe_sdk");

    // JSONL log: the write was appended.
    let log_path = dir.path().join("log.jsonl");
    let log = std::fs::read_to_string(&log_path).expect("log.jsonl should exist");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one log line for one claim");

    let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["kind"], "add_claim");
    assert_eq!(entry["subject"], "auth_service");
    assert_eq!(entry["predicate"], "depends_on");
    assert_eq!(entry["object"], "stripe_sdk");
    assert_eq!(entry["source"], "test_session");
}
