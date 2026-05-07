use memory_core::{AgentKind, Store};

#[test]
fn record_event_persists_episode_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "user_message",
            "Remember that AML should batch memory extraction.",
            "conversation",
        )
        .unwrap();

    let event = store.get_event(event_id).unwrap();

    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.kind, "user_message");
    assert_eq!(
        event.payload,
        "Remember that AML should batch memory extraction."
    );
    assert_eq!(event.source, "conversation");
    assert_eq!(event.agent_id, "local");
    assert_eq!(event.agent_kind, AgentKind::Human);
    assert!(event.ts > 0);
}
