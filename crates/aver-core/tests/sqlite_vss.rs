//! ADR-0017: the bundled `sqlite-vec` extension is the default vector
//! index. The legacy `sqlite-vss` capability probe is retained under a
//! renamed surface (`sqlite_vec_status`) and the `vec0` virtual table is
//! created by migration 0010 on every fresh open.

use aver_core::{SqliteVecStatus, Store};

#[test]
fn sqlite_vec_capability_is_available_with_bundled_extension() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let status = store.sqlite_vec_status().unwrap();

    assert_eq!(status, SqliteVecStatus::Available);
}

#[test]
fn vector_index_table_is_created_by_migration() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    assert!(
        store.vector_index_table_exists().unwrap(),
        "migration 0010 should create the vec0 virtual table on fresh open"
    );
    assert_eq!(store.vector_index_row_count().unwrap(), 0);
}
