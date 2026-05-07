//! T13 — v0.2 storage starts with a chunk table that can be joined to the
//! future sqlite-vss index. ADR-0006 keeps vector storage embedded in SQLite.

use aver_core::Store;

#[test]
fn fresh_database_has_vector_chunks_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("open should succeed");

    assert!(
        store.has_table("vector_chunks"),
        "vector_chunks table should exist after migrations"
    );
}
