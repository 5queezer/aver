use aver_core::Store;

fn open_store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap()
}

#[test]
fn claude_session_is_a_process() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeSession", "Process").unwrap());
}

#[test]
fn claude_session_is_a_thing_transitive() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeSession", "Thing").unwrap());
}

#[test]
fn claude_event_is_a_concept() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeEvent", "Concept").unwrap());
}

#[test]
fn claude_content_is_an_asset() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeContent", "Asset").unwrap());
}

#[test]
fn project_is_an_asset() {
    let store = open_store();
    assert!(store.entity_type_is_a("Project", "Asset").unwrap());
}

#[test]
fn project_path_is_a_file() {
    let store = open_store();
    assert!(store.entity_type_is_a("ProjectPath", "File").unwrap());
}

#[test]
fn claude_history_is_an_asset() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeHistory", "Asset").unwrap());
}

#[test]
fn claude_history_file_is_a_file() {
    let store = open_store();
    assert!(store.entity_type_is_a("ClaudeHistoryFile", "File").unwrap());
}

#[test]
fn infer_entity_type_uses_claude_session_prefix() {
    // infer_entity_type_name is private, so we exercise it via add_claim +
    // entity_type_name which goes through ensure_entity -> infer_entity_type_name.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim(
            "ClaudeSession:abc123",
            "relates_to",
            "Project:myapp",
            "test",
        )
        .unwrap();
    let type_name = store.entity_type_name("ClaudeSession:abc123").unwrap();
    assert_eq!(type_name, "ClaudeSession");
}

#[test]
fn entity_is_a_type_process_for_claude_session_entity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim(
            "ClaudeSession:abc123",
            "relates_to",
            "Project:myapp",
            "test",
        )
        .unwrap();
    assert!(
        store
            .entity_is_a_type("ClaudeSession:abc123", "Process")
            .unwrap()
    );
}
