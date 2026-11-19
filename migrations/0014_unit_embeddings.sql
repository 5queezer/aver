-- ADR-0017 amendment: vector projections store unit-length embeddings.
-- Store::open rewrites vector_chunks.embedding_json with the shared Rust
-- normalization boundary, then rebuilds this derived index from those rows.
DELETE FROM vector_index;
