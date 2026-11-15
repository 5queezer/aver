use aver_core::{EventWrite, Store};
use aver_server::tools::{AverTools, GetEventParams};

#[test]
fn get_event_reads_only_the_exact_requested_scope_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let id = store
        .record_event(
            EventWrite::new(
                "synthetic",
                "decision",
                "Invented private rationale.",
                "synthetic",
            )
            .scope("proj/sample"),
        )
        .unwrap();
    store.close().unwrap();
    let before = std::fs::read(dir.path().join("events.jsonl")).unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let view = tools
        .get_event(GetEventParams {
            event_id: id,
            scope: Some("proj/sample".into()),
        })
        .unwrap();
    assert_eq!(view.id, id);
    assert_eq!(view.payload, "Invented private rationale.");
    assert_eq!(view.scope, "proj/sample");
    let missing = tools
        .get_event(GetEventParams {
            event_id: id + 1,
            scope: Some("proj/sample".into()),
        })
        .unwrap_err()
        .to_string();
    let outside = tools
        .get_event(GetEventParams {
            event_id: id,
            scope: Some("proj/other".into()),
        })
        .unwrap_err()
        .to_string();
    assert_eq!(missing, outside);
    assert!(missing.contains("record_event"));
    assert!(!outside.contains("Invented private rationale"));
    assert!(
        tools
            .get_event(GetEventParams {
                event_id: id,
                scope: None
            })
            .is_err()
    );
    assert_eq!(
        std::fs::read(dir.path().join("events.jsonl")).unwrap(),
        before
    );
    assert!(!dir.path().join("log.jsonl").exists());
}
