//! ADR-0017: end-to-end checks for the `vec0` ANN index.
//!
//! - Fresh install: virtual table exists, rows arrive when matching-dim
//!   embeddings are inserted, KNN returns the nearest neighbour first.
//! - Upgrade install: rows present in `vector_chunks` before migration are
//!   backfilled into `vector_index` for matching dim, skipped otherwise.
//! - HybridRAG: `recall_hybrid_claims` produces the same ordering with the
//!   new path (verified by the existing hybrid_retrieval suite).

use aver_core::{Store, VECTOR_INDEX_DIM, vector::MockEmbeddingClient};

fn make_embedding(seed: f32) -> Vec<f32> {
    // Synthetic VECTOR_INDEX_DIM-long vector. The first slot dominates so
    // KNN nearest-neighbour ordering is predictable.
    let mut v = vec![0.0f32; VECTOR_INDEX_DIM];
    v[0] = seed;
    v[1] = 1.0 - seed;
    v
}

#[test]
fn matching_dim_embeddings_populate_vec_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    let client = MockEmbeddingClient::new(make_embedding(1.0));
    store
        .add_embedded_vector_chunk_for_claim(claim, "nomic-embed-text", &client)
        .unwrap();

    assert_eq!(store.vector_index_row_count().unwrap(), 1);
}

#[test]
fn off_dim_embeddings_skip_vec_index_but_persist_in_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim = store.add_claim("Aver", "uses", "SQLite", "test").unwrap();
    // 3-dim mock — does not match VECTOR_INDEX_DIM = 768.
    let client = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);
    store
        .add_embedded_vector_chunk_for_claim(claim, "nomic-embed-text", &client)
        .unwrap();

    assert_eq!(store.vector_index_row_count().unwrap(), 0);
    let (indexed, total) = store.vector_chunk_embedding_status().unwrap();
    assert_eq!((indexed, total), (1, 1));
}

#[test]
fn knn_query_returns_nearest_neighbour_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    // Three synthetic claims with embeddings spaced along the first axis.
    let near = store.add_claim("X", "is", "Near", "test").unwrap();
    let mid = store.add_claim("X", "is", "Mid", "test").unwrap();
    let far = store.add_claim("X", "is", "Far", "test").unwrap();
    for (claim, seed) in [(near, 1.0), (mid, 0.5), (far, 0.0)] {
        let client = MockEmbeddingClient::new(make_embedding(seed));
        store
            .add_embedded_vector_chunk_for_claim(claim, "nomic-embed-text", &client)
            .unwrap();
    }
    assert_eq!(store.vector_index_row_count().unwrap(), 3);

    // Query closest to seed=1.0 — `near` should rank first.
    let query_client = MockEmbeddingClient::new(make_embedding(1.0));
    let results = store
        .recall_hybrid_claims("Near", &query_client, 3)
        .unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].id, near, "nearest neighbour should rank first");
}

#[test]
fn migration_backfills_existing_vector_chunks_rows() {
    use rusqlite::Connection;

    // Simulate an "old" database: open once to create the schema,
    // then drop the vector_index table, insert raw chunks, and re-open.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    {
        let store = Store::open(dir.path()).unwrap();
        drop(store);
    }

    // Pretend we are at schema version 9 (pre-0010): drop the index and
    // rewind user_version so the migration runner re-applies 0010.
    // Note: migration 0011 uses non-idempotent `ALTER TABLE ADD COLUMN`,
    // so we drop those columns/tables here to let the gated re-run
    // recreate them cleanly.
    {
        let conn = Connection::open(&db_path).unwrap();
        // Drop everything migrations >=0010 created so the gated re-run
        // recreates them cleanly. Migration 0011 uses non-idempotent
        // `ALTER TABLE ADD COLUMN`, so the columns it added are dropped
        // too (requires SQLite >= 3.35).
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS claims_predicate_in_ontology_insert;
             DROP TRIGGER IF EXISTS claims_predicate_in_ontology_update;
             DROP TABLE IF EXISTS vector_index;
             DROP TABLE IF EXISTS predicate_alias;
             DROP TABLE IF EXISTS ontology_extension_log;
             ALTER TABLE entities DROP COLUMN requires_review;
             ALTER TABLE predicate_types DROP COLUMN created_via;
             ALTER TABLE predicate_types DROP COLUMN created_at;
             -- ADR-0021 migration 0084 adds non-idempotent ALTER TABLE ADD COLUMN
             -- statements + indexes + triggers that reference the new column.
             -- Drop indexes and triggers first; SQLite refuses to drop a column
             -- that is still referenced by an index.
             DROP INDEX IF EXISTS claims_scope;
             DROP INDEX IF EXISTS episodic_events_scope;
             DROP INDEX IF EXISTS observations_scope;
             DROP INDEX IF EXISTS candidate_claims_scope;
             DROP TRIGGER IF EXISTS claims_scope_nonblank_insert;
             DROP TRIGGER IF EXISTS claims_scope_nonblank_update;
             DROP TRIGGER IF EXISTS claims_scope_charset_insert;
             DROP TRIGGER IF EXISTS claims_scope_charset_update;
             DROP TRIGGER IF EXISTS episodic_events_scope_nonblank_insert;
             DROP TRIGGER IF EXISTS episodic_events_scope_nonblank_update;
             DROP TRIGGER IF EXISTS episodic_events_scope_charset_insert;
             DROP TRIGGER IF EXISTS episodic_events_scope_charset_update;
             DROP TRIGGER IF EXISTS observations_scope_nonblank_insert;
             DROP TRIGGER IF EXISTS observations_scope_nonblank_update;
             DROP TRIGGER IF EXISTS observations_scope_charset_insert;
             DROP TRIGGER IF EXISTS observations_scope_charset_update;
             DROP TRIGGER IF EXISTS candidate_claims_scope_nonblank_insert;
             DROP TRIGGER IF EXISTS candidate_claims_scope_nonblank_update;
             DROP TRIGGER IF EXISTS candidate_claims_scope_charset_insert;
             DROP TRIGGER IF EXISTS candidate_claims_scope_charset_update;
             ALTER TABLE claims DROP COLUMN scope;
             ALTER TABLE episodic_events DROP COLUMN scope;
             ALTER TABLE observations DROP COLUMN scope;
             ALTER TABLE candidate_claims DROP COLUMN scope;",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 9i64).unwrap();
        // Insert one matching-dim row and one off-dim row directly.
        let matching: Vec<f32> = {
            let mut v = vec![0.0; VECTOR_INDEX_DIM];
            v[0] = 1.0;
            v
        };
        let matching_json = serde_json::to_string(&matching).unwrap();
        // Need a claim row first; use minimal direct insert.
        conn.execute(
            "INSERT INTO claims (subject, predicate, object, source_refs, agent_id, agent_kind, provenance, confidence, status, write_ts, created_at, last_seen_at)
             VALUES ('S','relates_to','O','[\"t\"]','t','HUMAN','USER_ASSERTED',1.0,'ACTIVE',1,1,1)",
            [],
        )
        .unwrap();
        let claim_id: i64 = conn
            .query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 't', 'nomic-embed-text', ?2, 1)",
            rusqlite::params![claim_id, matching_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, 't', 'nomic-embed-text', '[1,0,0]', 1)",
            rusqlite::params![claim_id],
        )
        .unwrap();
    }

    // Re-open: migration 0010 re-runs and backfills only the matching-dim row.
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.vector_index_row_count().unwrap(), 1);
}
