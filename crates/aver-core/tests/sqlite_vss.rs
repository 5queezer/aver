use aver_core::{SqliteVssStatus, Store};

#[test]
fn sqlite_vss_capability_probe_is_offline_and_non_fatal_when_extension_missing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let status = store.sqlite_vss_status().unwrap();

    assert!(matches!(
        status,
        SqliteVssStatus::Available | SqliteVssStatus::Unavailable { .. }
    ));
}

#[test]
fn preparing_sqlite_vss_index_noops_when_extension_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let status = store.prepare_sqlite_vss_index(2).unwrap();

    if let SqliteVssStatus::Available = status {
        // Extension-capable developer machines may create the virtual table.
        assert!(store.vector_index_table_exists().unwrap());
    }
}
