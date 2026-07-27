//! Hyperedge write/read/traverse.

use rusqlite::params;

use crate::error::Error;
use crate::log::{HyperedgeLogEntry, append_jsonl};
use crate::privacy::privacy_filter;
use crate::store::Store;
use crate::types::{Hyperedge, HyperedgeInput, HyperedgeParticipant};
use crate::validation::validate_recall_query;

pub(crate) fn validate_hyperedge_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(Error::InvalidHyperedgeField { field });
    }
    Ok(())
}

pub(crate) fn validate_hyperedge_participant_field(
    field: &'static str,
    value: &str,
) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(Error::InvalidHyperedgeParticipant { field });
    }
    Ok(())
}

impl Store {
    pub fn add_hyperedge(&self, input: HyperedgeInput) -> Result<i64, Error> {
        validate_hyperedge_field("predicate", &input.predicate)?;
        if input.source_refs.is_empty() {
            return Err(Error::InvalidHyperedgeField {
                field: "source_refs",
            });
        }
        for source_ref in &input.source_refs {
            validate_hyperedge_field("source_refs", source_ref)?;
        }
        if input.participants.is_empty() {
            return Err(Error::InvalidHyperedgeParticipant { field: "role" });
        }
        for participant in &input.participants {
            validate_hyperedge_participant_field("role", &participant.role)?;
            validate_hyperedge_participant_field("entity", &participant.entity)?;
        }
        if !(0.0..=1.0).contains(&input.confidence) {
            return Err(Error::InvalidConfidence {
                value: input.confidence,
            });
        }

        let mut privacy_content = format!(
            "{} {} {}",
            input.predicate,
            input.provenance.as_str(),
            input.confidence
        );
        for source_ref in &input.source_refs {
            privacy_content.push(' ');
            privacy_content.push_str(source_ref);
        }
        for participant in &input.participants {
            privacy_content.push(' ');
            privacy_content.push_str(&participant.role);
            privacy_content.push(' ');
            privacy_content.push_str(&participant.entity);
        }
        if let Err(rejection) = privacy_filter(&privacy_content) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        for source_ref in &input.source_refs {
            self.privacy_filter_path_recording(source_ref)?;
        }
        for participant in &input.participants {
            self.privacy_filter_path_recording(&participant.entity)?;
        }

        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let extend_ontology = self.validate_ontology(&input.predicate, input.provenance)?;

        // BEGIN IMMEDIATE covers id allocation + log append + projection so
        // concurrent processes cannot allocate duplicate hyperedge ids. The
        // log append remains strictly before the SQLite INSERTs (ADR-0005).
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let hyperedge_id: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM hyperedges",
                [],
                |row| row.get(0),
            )?;

            let entry = HyperedgeLogEntry {
                kind: "add_hyperedge",
                ts: now,
                hyperedge_id,
                predicate: &input.predicate,
                provenance: input.provenance.as_str(),
                confidence: input.confidence,
                source_refs: &input.source_refs,
                participants: &input.participants,
            };
            append_jsonl(&self.log_path, &entry)?;

            if extend_ontology {
                self.apply_ontology_extension(&input.predicate, "local", now)?;
            }

            let source_refs_json = serde_json::to_string(&input.source_refs)?;
            self.conn.execute(
                "INSERT INTO hyperedges (id, predicate, provenance, confidence, source_refs, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ACTIVE', ?6, ?6)",
                params![
                    hyperedge_id,
                    input.predicate,
                    input.provenance.as_str(),
                    input.confidence,
                    source_refs_json,
                    now,
                ],
            )?;
            for participant in &input.participants {
                self.ensure_entity(&participant.entity, now)?;
                self.conn.execute(
                    "INSERT INTO hyperedge_participants (hyperedge_id, role, entity)
                     VALUES (?1, ?2, ?3)",
                    params![hyperedge_id, participant.role, participant.entity],
                )?;
            }
            Ok(hyperedge_id)
        })();
        match result {
            Ok(hyperedge_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(hyperedge_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn get_hyperedge(&self, id: i64) -> Result<Hyperedge, Error> {
        let (id, predicate, provenance, confidence, source_refs, status, created_at, updated_at): (
            i64,
            String,
            String,
            f64,
            String,
            String,
            i64,
            i64,
        ) = self.conn.query_row(
            "SELECT id, predicate, provenance, confidence, source_refs, status, created_at, updated_at
               FROM hyperedges
              WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        )?;
        Ok(Hyperedge {
            id,
            predicate,
            provenance: provenance.parse()?,
            confidence,
            source_refs: serde_json::from_str(&source_refs)?,
            status: status.parse()?,
            created_at,
            updated_at,
            participants: self.hyperedge_participants(id)?,
        })
    }

    pub fn list_active_hyperedges(&self) -> Result<Vec<Hyperedge>, Error> {
        self.active_hyperedges_matching(None)
    }

    pub fn recall_hyperedges(&self, query: &str) -> Result<Vec<Hyperedge>, Error> {
        validate_recall_query(query)?;
        self.active_hyperedges_matching(Some(query))
    }

    pub fn traverse_hyperedges(&self, entity: &str) -> Result<Vec<Hyperedge>, Error> {
        validate_hyperedge_participant_field("entity", entity)?;
        self.active_hyperedges_matching(Some(entity))
    }

    pub(crate) fn active_hyperedges_matching(
        &self,
        query: Option<&str>,
    ) -> Result<Vec<Hyperedge>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM hyperedges
              WHERE status = 'ACTIVE'
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let needle = query.map(|q| q.to_ascii_lowercase());
        let mut edges = Vec::new();
        for id in ids {
            let edge = self.get_hyperedge(id)?;
            let matches = needle.as_ref().is_none_or(|needle| {
                edge.predicate.to_ascii_lowercase().contains(needle)
                    || edge
                        .source_refs
                        .iter()
                        .any(|source| source.to_ascii_lowercase().contains(needle))
                    || edge.participants.iter().any(|participant| {
                        participant.role.to_ascii_lowercase().contains(needle)
                            || participant.entity.to_ascii_lowercase().contains(needle)
                    })
            });
            if matches {
                edges.push(edge);
            }
        }
        Ok(edges)
    }

    pub(crate) fn hyperedge_participants(
        &self,
        hyperedge_id: i64,
    ) -> Result<Vec<HyperedgeParticipant>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, hyperedge_id, role, entity
               FROM hyperedge_participants
              WHERE hyperedge_id = ?1
              ORDER BY id",
        )?;
        let participants = stmt
            .query_map([hyperedge_id], |row| {
                Ok(HyperedgeParticipant {
                    id: row.get(0)?,
                    hyperedge_id: row.get(1)?,
                    role: row.get(2)?,
                    entity: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(participants)
    }
}
