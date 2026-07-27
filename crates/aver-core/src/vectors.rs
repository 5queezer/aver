//! Vector chunks and hybrid recall.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, types::Type};

use crate::error::Error;
use crate::recall::graph_score_for_query_claim;
use crate::retrieval;
use crate::store::Store;
use crate::store::VECTOR_INDEX_DIM;
use crate::types::{Claim, ClaimStatus, VectorChunk};
use crate::validation::{
    validate_embedding_model, validate_embedding_vector, validate_recall_query, validate_top_k,
    validate_vector_chunk_text,
};
use crate::vector;

pub(crate) fn parse_optional_embedding(
    value: Option<String>,
) -> rusqlite::Result<Option<Vec<f32>>> {
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
            })
        })
        .transpose()
}

impl Store {
    /// Insert vector chunk metadata for a claim. The `sqlite-vec`/`vec0` ANN
    /// table is maintained separately; this table is the durable join point
    /// between claims and embeddings.
    pub fn add_vector_chunk(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
        self.privacy_filter_recording(text)?;
        self.privacy_filter_path_recording(embedding_model)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![claim_id, text, embedding_model, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert vector chunk metadata with its embedding vector serialized for
    /// deterministic local storage. When the embedding's dimension matches
    /// the canonical [`VECTOR_INDEX_DIM`], the same vector is also written
    /// to the `vec0` ANN index in the same transaction
    /// (ADR-0017 §"Populate strategy"). Off-dimension rows stay only in
    /// `vector_chunks`; recall covers them via the JSON full-scan fallback.
    pub fn add_vector_chunk_with_embedding(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
        embedding: &[f32],
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
        validate_embedding_vector(embedding)?;
        self.privacy_filter_recording(text)?;
        self.privacy_filter_path_recording(embedding_model)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let embedding_json = serde_json::to_string(embedding)?;

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![claim_id, text, embedding_model, embedding_json, now],
        )?;
        let chunk_id = tx.last_insert_rowid();
        if embedding.len() == VECTOR_INDEX_DIM && self.has_table("vector_index") {
            tx.execute(
                "INSERT INTO vector_index(chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, embedding_json],
            )?;
        }
        tx.commit()?;
        Ok(chunk_id)
    }

    pub(crate) fn ensure_claim_exists(&self, claim_id: i64) -> Result<(), Error> {
        self.get_claim(claim_id)?;
        Ok(())
    }

    /// Insert vector chunk metadata using the canonical claim text rendering.
    pub fn add_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        let claim = self.get_claim(claim_id)?;
        self.add_vector_chunk(claim_id, &claim.text(), embedding_model)
    }

    /// Embed the canonical claim text and persist the resulting vector chunk.
    pub fn add_embedded_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
        client: &impl vector::EmbeddingClient,
    ) -> Result<i64, Error> {
        let claim = self.get_claim(claim_id)?;
        let text = claim.text();
        let embedding = client.embed(&text)?;
        self.add_vector_chunk_with_embedding(claim_id, &text, embedding_model, &embedding)
    }

    /// Retrieve vector chunk metadata by id.
    pub fn get_vector_chunk(&self, id: i64) -> Result<VectorChunk, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, text, embedding_model, embedding_json FROM vector_chunks WHERE id = ?1",
                [id],
                |row| {
                    let embedding_json: Option<String> = row.get(4)?;
                    Ok(VectorChunk {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        text: row.get(2)?,
                        embedding_model: row.get(3)?,
                        embedding: parse_optional_embedding(embedding_json)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingVectorChunk { chunk_id: id },
                other => Error::Sqlite(other),
            })
    }

    /// List vector chunk metadata for a claim in stable insertion order.
    pub fn list_vector_chunks_for_claim(&self, claim_id: i64) -> Result<Vec<VectorChunk>, Error> {
        self.ensure_claim_exists(claim_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let rows = stmt.query_map([claim_id], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut chunks = Vec::new();
        for row in rows {
            chunks.push(row?);
        }
        Ok(chunks)
    }

    /// Returns how many vector_chunks have non-null embeddings vs total.
    pub fn vector_chunk_embedding_status(&self) -> Result<(usize, usize), Error> {
        let total: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM vector_chunks", [], |row| row.get(0))?;
        let indexed: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM vector_chunks WHERE embedding_json IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok((indexed, total))
    }

    /// Backfill a bounded batch of stored embeddings. The default batch keeps
    /// maintenance calls resumable instead of loading the whole backlog.
    pub fn backfill_vector_embeddings(
        &self,
        client: &dyn crate::vector::EmbeddingClient,
    ) -> Result<usize, Error> {
        self.backfill_vector_embeddings_with_limit(client, 100)
    }

    /// Backfill at most `limit` chunks with missing embeddings. Individual
    /// provider failures are skipped so successful rows remain durable and the
    /// next invocation can resume from the remaining NULL rows.
    pub fn backfill_vector_embeddings_with_limit(
        &self,
        client: &dyn crate::vector::EmbeddingClient,
        limit: usize,
    ) -> Result<usize, Error> {
        if limit == 0 {
            return Ok(0);
        }
        let sql_limit = limit.min(i64::MAX as usize) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, text
               FROM vector_chunks
              WHERE embedding_json IS NULL
              ORDER BY id
              LIMIT ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([sql_limit], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut count = 0;
        let index_present = self.has_table("vector_index");
        for (id, text) in rows {
            let Ok(embedding) = client.embed(&text) else {
                continue;
            };
            let embedding_json = serde_json::to_string(&embedding)?;
            self.conn.execute(
                "UPDATE vector_chunks SET embedding_json = ?1 WHERE id = ?2",
                params![embedding_json, id],
            )?;
            // ADR-0017: keep `vector_index` in sync for matching-dim rows.
            // `INSERT OR IGNORE` makes the call idempotent if the row was
            // already backfilled by migration 0010.
            if index_present && embedding.len() == VECTOR_INDEX_DIM {
                self.conn.execute(
                    "INSERT OR IGNORE INTO vector_index(chunk_id, embedding) VALUES (?1, ?2)",
                    params![id, embedding_json],
                )?;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Recall claims ranked by cosine similarity to the query embedding,
    /// combined with text-search results (best score per claim_id wins).
    pub fn recall_text_with_embedding(
        &self,
        query: &str,
        client: &dyn crate::vector::EmbeddingClient,
    ) -> Result<Vec<Claim>, Error> {
        let query_embedding = client.embed(query)?;

        // Score each claim that has a stored embedding.
        let mut scores: HashMap<i64, f64> = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT claim_id, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (claim_id, embedding_json) = row?;
            if let Some(embedding) = parse_optional_embedding(embedding_json)?
                && let Some(score) = vector::normalized_cosine_score(&query_embedding, &embedding)
            {
                scores
                    .entry(claim_id)
                    .and_modify(|current| *current = current.max(f64::from(score)))
                    .or_insert(f64::from(score));
            }
        }
        drop(stmt);

        // Merge with text-search results.
        let text_claims = self.recall_text(query)?;
        let text_score_base = 0.5_f64;
        for claim in &text_claims {
            scores
                .entry(claim.id)
                .and_modify(|current| *current = current.max(text_score_base))
                .or_insert(text_score_base);
        }

        let mut candidates = Vec::with_capacity(scores.len());
        for claim_id in scores.keys().copied() {
            let claim = self.get_claim(claim_id)?;
            if claim.status == ClaimStatus::Active {
                candidates.push((scores[&claim_id], claim));
            }
        }
        candidates.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        Ok(candidates.into_iter().map(|(_, c)| c).collect())
    }

    /// Rank persisted vector chunks by normalized cosine similarity to the
    /// query embedding, returning the best matches first.
    pub fn rank_vector_chunks_by_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        validate_top_k(top_k)?;
        validate_embedding_vector(query_embedding)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut scored = Vec::new();
        for row in rows {
            let chunk = row?;
            if let Some(embedding) = &chunk.embedding
                && let Some(score) = vector::normalized_cosine_score(query_embedding, embedding)
            {
                scored.push((score, chunk));
            }
        }
        scored.sort_by(|(a_score, a_chunk), (b_score, b_chunk)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_chunk.id.cmp(&b_chunk.id))
        });
        scored.truncate(top_k);
        Ok(scored.into_iter().map(|(_, chunk)| chunk).collect())
    }

    /// Embed a query with the provided client, then rank persisted vector
    /// chunks by similarity. Tests use `MockEmbeddingClient`; production can
    /// pass `OllamaEmbeddingClient` without changing storage logic.
    pub fn recall_vector_chunks(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;

        let query_embedding = client.embed(query)?;
        self.rank_vector_chunks_by_embedding(&query_embedding, top_k)
    }

    /// Vector recall that returns claim rows instead of internal chunk
    /// metadata, preserving the chunk ranking order.
    pub fn recall_vector_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let chunks = self.recall_vector_chunks(query, client, usize::MAX)?;
        let mut seen = HashSet::new();
        let mut claims = Vec::new();
        for chunk in chunks {
            if seen.insert(chunk.claim_id) {
                let claim = self.get_claim(chunk.claim_id)?;
                if claim.status != ClaimStatus::Active {
                    continue;
                }
                claims.push(claim);
                if claims.len() == top_k {
                    break;
                }
            }
        }
        Ok(claims)
    }

    /// Hybrid recall over vector chunks plus text fallback. Vector-ranked
    /// claims are returned first; text recall fills any remaining slots with
    /// distinct claims so sparse vector indexes remain useful.
    pub fn recall_hybrid_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        self.recall_hybrid_claims_with_alpha(
            query,
            client,
            top_k,
            retrieval::HybridWeights::for_query(query),
        )
    }

    pub fn recall_hybrid_claims_with_alpha(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
        weights: retrieval::HybridWeights,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;
        let query_embedding = client.embed(query)?;

        // ADR-0017: vector half. When the query embedding matches the
        // canonical dimension and the `vec0` index is present, run a single
        // KNN MATCH instead of an O(N) scan + per-row cosine in Rust. The
        // fanout (`top_k * 4`) gives the graph half room to re-rank without
        // starving on vector-only candidates. Distance is L2 over (assumed)
        // L2-normalised embeddings; we map to a similarity score in [0, 1]
        // via `1 - distance / 2` then clamp, matching the cosine-derived
        // range produced by `normalized_cosine_score`. Off-dimension queries
        // and unindexed installs fall back to the JSON full-scan path.
        let mut vector_scores: HashMap<i64, f64> = HashMap::new();
        let use_vec_index =
            query_embedding.len() == VECTOR_INDEX_DIM && self.has_table("vector_index");
        if use_vec_index {
            let fanout = top_k.saturating_mul(4).max(top_k).min(i64::MAX as usize);
            let q_json = serde_json::to_string(&query_embedding)?;
            let mut stmt = self.conn.prepare(
                "SELECT vc.claim_id, vi.distance
                   FROM vector_index vi
                   JOIN vector_chunks vc ON vc.id = vi.chunk_id
                  WHERE vi.embedding MATCH ?1
                    AND k = ?2
                  ORDER BY vi.distance",
            )?;
            let rows = stmt.query_map(params![q_json, fanout as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (claim_id, distance) = row?;
                let similarity = (1.0 - distance / 2.0).clamp(0.0, 1.0);
                vector_scores
                    .entry(claim_id)
                    .and_modify(|current| *current = current.max(similarity))
                    .or_insert(similarity);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT claim_id, embedding_json
                   FROM vector_chunks
                  WHERE embedding_json IS NOT NULL
                  ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (claim_id, embedding_json) = row?;
                if let Some(embedding) = parse_optional_embedding(embedding_json)?
                    && let Some(score) =
                        vector::normalized_cosine_score(&query_embedding, &embedding)
                {
                    vector_scores
                        .entry(claim_id)
                        .and_modify(|current: &mut f64| *current = current.max(f64::from(score)))
                        .or_insert(f64::from(score));
                }
            }
        }

        let text_claims = self.recall_text(query)?;
        let mut candidate_ids: HashSet<i64> = text_claims.iter().map(|claim| claim.id).collect();
        candidate_ids.extend(vector_scores.keys().copied());

        let mut candidates = Vec::new();
        for claim_id in candidate_ids {
            let claim = self.get_claim(claim_id)?;
            if claim.status != ClaimStatus::Active {
                continue;
            }
            let vector_score = vector_scores.get(&claim_id).copied().unwrap_or(0.0);
            let graph_score = graph_score_for_query_claim(query, &claim);
            candidates.push((weights.blend(vector_score, graph_score), claim));
        }
        candidates.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        candidates.truncate(top_k);
        Ok(candidates.into_iter().map(|(_, claim)| claim).collect())
    }
}
