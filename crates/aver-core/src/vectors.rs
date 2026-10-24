//! Vector chunks, embeddings and hybrid recall.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, types::Type};

use crate::apply::apply;
use crate::claims::SqlParams;
use crate::error::Error;
use crate::log::{LogRecord, append_jsonl};
use crate::recall::graph_score_for_query_claim;
use crate::retrieval;
use crate::store::{Store, VECTOR_INDEX_DIM};
use crate::types::{Claim, ClaimStatus, RecallFilters, RecallQuery, VectorChunk};
use crate::validation::{
    validate_embedding_model, validate_embedding_vector, validate_recall_query, validate_top_k,
    validate_vector_chunk_text,
};
use crate::vector;
impl Store {
    /// Insert vector chunk metadata for a claim without an embedding (to be
    /// backfilled by [`Store::backfill_vector_embeddings`]). `vector_chunks`
    /// is the durable join point between claims and embeddings; the chunk is
    /// logged (ADR-0026) so replay rebuilds it.
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
        self.insert_vector_chunk(claim_id, text, embedding_model, None)
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
        self.insert_vector_chunk(claim_id, text, embedding_model, Some(embedding.to_vec()))
    }

    /// Log-first chunk write (ADR-0005/0026): mint the id, append the record
    /// to the claims log (chunks reference claims, which replay first), then
    /// apply inside one write transaction.
    fn insert_vector_chunk(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
        embedding: Option<Vec<f32>>,
    ) -> Result<i64, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let chunk_id = self.ids.mint();
            let record = LogRecord::AddVectorChunk {
                ts: now,
                chunk_id,
                claim_id,
                text: text.to_string(),
                embedding_model: embedding_model.to_string(),
                embedding,
            };
            append_jsonl(&self.log_path, &record)?;
            apply(&self.conn, &record)?;
            Ok(chunk_id)
        })();
        match result {
            Ok(chunk_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(chunk_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
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
        self.ensure_claim_exists(claim_id)?;
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
        self.ensure_claim_exists(claim_id)?;
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

    /// Backfill stored embeddings for any vector_chunks that have no embedding yet,
    /// using the provided EmbeddingClient. Each backfilled embedding is logged
    /// as an `embed_vector_chunk` record (ADR-0026) so replay rebuilds the
    /// same `vector_chunks` and `vector_index` rows. Returns how many were
    /// backfilled.
    pub fn backfill_vector_embeddings(
        &self,
        client: &dyn crate::vector::EmbeddingClient,
    ) -> Result<usize, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text FROM vector_chunks WHERE embedding_json IS NULL ORDER BY id",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        let mut count = 0;
        for (chunk_id, text) in rows {
            let embedding = client.embed(&text)?;
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            let result = (|| -> Result<(), Error> {
                let record = LogRecord::EmbedVectorChunk {
                    ts: now,
                    chunk_id,
                    embedding,
                };
                append_jsonl(&self.log_path, &record)?;
                apply(&self.conn, &record)?;
                Ok(())
            })();
            match result {
                Ok(()) => self.conn.execute_batch("COMMIT")?,
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
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
        let text_claims = self.recall_text(RecallQuery::new(query))?;
        let text_score_base = 0.5_f64;
        for claim in &text_claims {
            scores
                .entry(claim.id)
                .and_modify(|current| *current = current.max(text_score_base))
                .or_insert(text_score_base);
        }

        let mut candidates: Vec<(f64, Claim)> = scores
            .keys()
            .copied()
            .filter_map(|claim_id| {
                self.get_claim(claim_id)
                    .ok()
                    .filter(|c| c.status == ClaimStatus::Active)
                    .map(|c| (scores[&claim_id], c))
            })
            .collect();
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

    /// ADR-0004 hybrid recall: the vector half plus the keyword half blended
    /// by `weights`, ACTIVE claims only, no other filters. Embeds the query
    /// with `client`; `top_k == 0` returns nothing without embedding.
    pub fn recall_hybrid_claims(
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
        let embedding = client.embed(query)?;
        self.recall_hybrid_claims_with_filters(
            query,
            &embedding,
            top_k,
            weights,
            &RecallFilters::default(),
        )
    }

    /// ADR-0004 hybrid recall with ADR-0021 scope and ADR-0023 typed filters
    /// pushed into candidate selection on both halves, for callers that
    /// already hold the query embedding (the server embeds once and ranks
    /// claims and chunks separately).
    ///
    /// Vector half (ADR-0017): when the embedding has the canonical
    /// dimension and the `vec0` index exists, one KNN MATCH over a fanout
    /// pool; otherwise the JSON full scan. Candidates from the pool are kept
    /// only if they pass `filters`; the keyword half runs with the same
    /// filters. Scores blend `weights.alpha` × vector similarity with the
    /// graph score; ties break by id.
    pub fn recall_hybrid_claims_with_filters(
        &self,
        query: &str,
        embedding: &[f32],
        top_k: usize,
        weights: retrieval::HybridWeights,
        filters: &RecallFilters,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;
        validate_embedding_vector(embedding)?;
        self.validate_filters(filters)?;

        let vector_scores = self.vector_scores(embedding, top_k)?;
        let Some((filter_sql, filter_params)) = self.claim_filter_sql(filters)? else {
            return Ok(Vec::new());
        };

        let mut candidates: HashMap<i64, Claim> = self
            .recall_text(RecallQuery::new(query).filters(filters.clone()))?
            .into_iter()
            .map(|claim| (claim.id, claim))
            .collect();
        let mut vector_ids: Vec<i64> = vector_scores
            .keys()
            .filter(|id| !candidates.contains_key(id))
            .copied()
            .collect();
        vector_ids.sort_unstable();
        if !vector_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", vector_ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let mut params: SqlParams = vector_ids
                .iter()
                .map(|id| Box::new(*id) as Box<dyn rusqlite::ToSql>)
                .collect();
            params.extend(filter_params);
            for claim in
                self.claims_where(&format!("id IN ({placeholders}) AND {filter_sql}"), params)?
            {
                candidates.insert(claim.id, claim);
            }
        }

        let mut ranked: Vec<(f64, Claim)> = candidates
            .into_values()
            .map(|claim| {
                let vector_score = vector_scores.get(&claim.id).copied().unwrap_or(0.0);
                let graph_score = graph_score_for_query_claim(query, &claim);
                (weights.blend(vector_score, graph_score), claim)
            })
            .collect();
        ranked.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        ranked.truncate(top_k);
        Ok(ranked.into_iter().map(|(_, claim)| claim).collect())
    }

    /// Best vector similarity per claim for `embedding`. ADR-0017: a single
    /// KNN MATCH over the `vec0` index when the dimension is canonical and
    /// the index exists; the fanout (`4 × top_k`, at least 64) leaves the
    /// graph half and the filters room to re-rank without starving on
    /// vector-only candidates. Distance is L2 over (assumed) L2-normalised
    /// embeddings, mapped to `1 - distance / 2` and clamped to [0, 1] to
    /// match `normalized_cosine_score`. Off-dimension queries and unindexed
    /// installs fall back to the JSON full scan.
    fn vector_scores(&self, embedding: &[f32], top_k: usize) -> Result<HashMap<i64, f64>, Error> {
        let mut vector_scores: HashMap<i64, f64> = HashMap::new();
        let use_vec_index = embedding.len() == VECTOR_INDEX_DIM && self.has_table("vector_index");
        if use_vec_index {
            let fanout = top_k.saturating_mul(4).max(64);
            let q_json = serde_json::to_string(embedding)?;
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
                if let Some(stored) = parse_optional_embedding(embedding_json)?
                    && let Some(score) = vector::normalized_cosine_score(embedding, &stored)
                {
                    vector_scores
                        .entry(claim_id)
                        .and_modify(|current: &mut f64| *current = current.max(f64::from(score)))
                        .or_insert(f64::from(score));
                }
            }
        }
        Ok(vector_scores)
    }
}

fn parse_optional_embedding(value: Option<String>) -> rusqlite::Result<Option<Vec<f32>>> {
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
            })
        })
        .transpose()
}
