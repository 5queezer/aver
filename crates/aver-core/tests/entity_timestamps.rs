use aver_core::{Store, replay};
use rusqlite::Connection;
use serde_json::json;

fn entity_times(dir: &std::path::Path) -> Vec<(String, i64, i64)> {
    let conn = Connection::open(dir.join("db.sqlite")).unwrap();
    conn.prepare("SELECT name, created_at, last_seen_at FROM entities ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn out_of_order_claim_and_hyperedge_records_preserve_monotonic_entity_times() {
    let dir = tempfile::tempdir().unwrap();
    let claim = |id, ts| json!({"kind":"add_claim","ts":ts,"claim_id":id,"subject":"sample","predicate":"uses","object":"widget","source":"synthetic","agent_id":"model","agent_kind":"LLM","confidence":0.45,"provenance":"INFERRED","scope":"global"});
    let hyperedge = |id, ts| json!({"kind":"add_hyperedge","ts":ts,"hyperedge_id":id,"predicate":"concerns","provenance":"EXTRACTED","confidence":0.9,"source_refs":["synthetic"],"participants":[{"role":"subject","entity":"sample"},{"role":"dependency","entity":"widget"}]});
    // Record order follows append/lock order; timestamps may follow capture
    // order, a skewed clock, or another writer's clock when logs are merged.
    let records = [
        claim(1, 20),
        claim(2, 10),
        hyperedge(3, 40),
        claim(4, 30),
        hyperedge(5, 25),
    ];
    let log = records
        .iter()
        .map(|record| format!("{record}\n"))
        .collect::<String>();
    std::fs::write(dir.path().join("log.jsonl"), &log).unwrap();
    let expected = vec![("sample".to_owned(), 20, 40), ("widget".to_owned(), 20, 40)];
    for force in [false, true, true] {
        replay(dir.path(), force).unwrap();
        assert_eq!(entity_times(dir.path()), expected);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.get_claim(1).unwrap().write_ts, 20);
        assert_eq!(store.get_claim(2).unwrap().write_ts, 10);
        assert_eq!(store.get_claim(4).unwrap().write_ts, 30);
        store.close().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap(),
            log
        );
    }
}
