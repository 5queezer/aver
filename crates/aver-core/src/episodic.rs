//! Episodic events, observations, coverage, pruning, and extraction triggers.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params, types::Type};

use crate::error::Error;
use crate::log::{EventLogEntry, ObservationLogEntry, ObservationPruneLogEntry, append_jsonl};
use crate::store::Store;
use crate::types::{
    AgentKind, ConsolidationReport, EpisodicEvent, ExtractionDecision, ExtractionTriggerReason,
    GraphDriftSnapshot, Observation, ObservationCoverage, ObservationDraft, ObservationRecall,
    ObservationRelevance, Provenance,
};
use crate::validation::{
    validate_agent_id, validate_event_field, validate_observation_field, validate_scope,
};

pub trait Observer {
    fn observe(&self, events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockObserver {
    drafts: Vec<ObservationDraft>,
}

impl MockObserver {
    pub fn new(drafts: Vec<ObservationDraft>) -> Self {
        Self { drafts }
    }
}

impl Observer for MockObserver {
    fn observe(&self, _events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error> {
        Ok(self.drafts.clone())
    }
}

pub(crate) fn observation_id(session_id: &str, content: &str, source_event_ids: &[i64]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in session_id
        .as_bytes()
        .iter()
        .chain([0xff].iter())
        .chain(content.as_bytes().iter())
        .chain([0xfe].iter())
        .chain(
            source_event_ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .collect::<Vec<_>>()
                .iter(),
        )
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    // Full 64-bit digest: truncating to 48 bits made collisions (which
    // INSERT OR REPLACE would silently overwrite with) meaningfully likely.
    format!("{hash:016x}")
}

pub(crate) fn observation_prune_marker_id(
    session_id: &str,
    pruned_observation_ids: &[String],
    ts: i64,
) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in session_id
        .as_bytes()
        .iter()
        .chain([0xfd].iter())
        .chain(ts.to_le_bytes().iter())
        .chain([0xfc].iter())
        .chain(
            pruned_observation_ids
                .iter()
                .flat_map(|id| id.as_bytes().iter().copied())
                .collect::<Vec<_>>()
                .iter(),
        )
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")[..12].to_string()
}

impl Store {
    pub(crate) fn agent_log_path(&self, agent_id: &str) -> Result<PathBuf, Error> {
        validate_agent_id(agent_id)?;
        Ok(self
            .log_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("agents")
            .join(agent_id)
            .join("log.jsonl"))
    }

    pub fn record_event(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.record_event_inner(
            "local",
            AgentKind::Human,
            session_id,
            kind,
            payload,
            source,
            "global",
        )
    }

    pub fn record_event_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.record_event_inner(
            agent_id, agent_kind, session_id, kind, payload, source, "global",
        )
    }

    /// ADR-0021: variant of `record_event` that records under an explicit scope.
    pub fn record_event_with_scope(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        self.record_event_inner(
            "local",
            AgentKind::Human,
            session_id,
            kind,
            payload,
            source,
            scope,
        )
    }

    /// ADR-0021: variant of `record_event_from_agent` that records under an
    /// explicit scope.
    #[allow(clippy::too_many_arguments)]
    pub fn record_event_from_agent_with_scope(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        self.record_event_inner(
            agent_id, agent_kind, session_id, kind, payload, source, scope,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_event_inner(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_event_field("session_id", session_id)?;
        validate_event_field("kind", kind)?;
        validate_event_field("source", source)?;
        validate_agent_id(agent_id)?;
        self.privacy_filter_recording(&format!(
            "{agent_id} {} {session_id} {kind} {payload} {source}",
            agent_kind.as_str()
        ))?;
        self.privacy_filter_path_recording(source)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // BEGIN IMMEDIATE serializes id allocation across processes (same
        // race class as insert_claim). Log append stays before the INSERT.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let event_id: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM episodic_events",
                [],
                |r| r.get(0),
            )?;
            let entry = EventLogEntry {
                kind: "record_event",
                ts: now,
                event_id,
                session_id,
                event_kind: kind,
                payload,
                source,
                agent_id,
                agent_kind: agent_kind.as_str(),
                scope,
            };
            append_jsonl(&self.event_log_path, &entry)?;
            self.conn.execute(
                "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event_id,
                    session_id,
                    kind,
                    payload,
                    source,
                    agent_id,
                    agent_kind.as_str(),
                    now,
                    scope,
                ],
            )?;
            Ok(event_id)
        })();
        match result {
            Ok(event_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(event_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn get_event(&self, id: i64) -> Result<EpisodicEvent, Error> {
        let (id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope): (
            i64,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
            String,
        ) = self
            .conn
            .query_row(
                "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope
               FROM episodic_events WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEvent { event_id: id },
                other => Error::Sqlite(other),
            })?;

        Ok(EpisodicEvent {
            id,
            session_id,
            kind,
            payload,
            source,
            agent_id,
            agent_kind: agent_kind.parse()?,
            ts,
            scope,
        })
    }

    pub fn list_events_for_session(&self, session_id: &str) -> Result<Vec<EpisodicEvent>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM episodic_events WHERE session_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(self.get_event(row?)?);
        }
        Ok(events)
    }

    pub fn record_observation(
        &self,
        session_id: &str,
        content: &str,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: &str,
    ) -> Result<String, Error> {
        self.record_observation_inner(
            session_id,
            content,
            relevance,
            source_event_ids,
            derivation,
            "global",
        )
    }

    /// ADR-0021: variant of `record_observation` that records under an explicit scope.
    pub fn record_observation_with_scope(
        &self,
        session_id: &str,
        content: &str,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: &str,
        scope: &str,
    ) -> Result<String, Error> {
        validate_scope(scope)?;
        self.record_observation_inner(
            session_id,
            content,
            relevance,
            source_event_ids,
            derivation,
            scope,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_observation_inner(
        &self,
        session_id: &str,
        content: &str,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: &str,
        scope: &str,
    ) -> Result<String, Error> {
        validate_event_field("session_id", session_id)?;
        validate_observation_field("content", content)?;
        validate_observation_field("derivation", derivation)?;
        if source_event_ids.is_empty() {
            return Err(Error::MissingEventProvenance { event_id: 0 });
        }
        self.privacy_filter_recording(&format!("{session_id} {content} {derivation}"))?;
        self.privacy_filter_path_recording(derivation)?;

        let mut events = Vec::new();
        for event_id in source_event_ids {
            let event = self.get_event(*event_id)?;
            if event.session_id != session_id {
                return Err(Error::MissingEventProvenance {
                    event_id: *event_id,
                });
            }
            events.push(event);
        }
        let first_event = events
            .first()
            .expect("source_event_ids is checked non-empty before event lookup");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let id = observation_id(session_id, content, source_event_ids);
        let source_event_ids_json = serde_json::to_string(source_event_ids)?;
        // Serialize the idempotency check, log append, and projection insert so
        // concurrent identical retries cannot both append or race the unique key.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<String, Error> {
            let existing: Option<(String, String, String)> = self
                .conn
                .query_row(
                    "SELECT session_id, content, source_event_ids FROM observations WHERE id = ?1",
                    [&id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((existing_session, existing_content, existing_event_ids)) = existing {
                if existing_session == session_id
                    && existing_content == content
                    && existing_event_ids == source_event_ids_json
                {
                    return Ok(id.clone());
                }
                return Err(Error::ObservationIdCollision {
                    observation_id: id.clone(),
                });
            }
            let entry = ObservationLogEntry {
                kind: "record_observation",
                ts: now,
                observation_id: &id,
                session_id,
                content,
                relevance: relevance.as_str(),
                source_event_ids,
                agent_id: &first_event.agent_id,
                agent_kind: first_event.agent_kind.as_str(),
                derivation,
                scope,
            };
            append_jsonl(&self.observation_log_path, &entry)?;
            self.conn.execute(
                "INSERT INTO observations
                 (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    session_id,
                    content,
                    relevance.as_str(),
                    source_event_ids_json,
                    first_event.agent_id,
                    first_event.agent_kind.as_str(),
                    derivation,
                    now,
                    scope,
                ],
            )?;
            Ok(id.clone())
        })();
        match result {
            Ok(id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn propose_observations_from_observer(
        &self,
        session_id: &str,
        observer: &impl Observer,
    ) -> Result<Vec<String>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = observer.observe(&events)?;
        let mut covered_event_ids = self
            .observation_coverage(session_id)?
            .covered_event_ids
            .into_iter()
            .collect::<HashSet<_>>();
        let mut ids = Vec::new();

        for draft in drafts {
            let uncovered_event_ids: Vec<i64> = draft
                .source_event_ids
                .into_iter()
                .filter(|event_id| covered_event_ids.insert(*event_id))
                .collect();
            if uncovered_event_ids.is_empty() {
                continue;
            }

            let id = self.record_observation(
                session_id,
                &draft.content,
                draft.relevance,
                &uncovered_event_ids,
                &draft.derivation,
            )?;
            ids.push(id);
        }
        Ok(ids)
    }

    pub fn get_observation(&self, id: &str) -> Result<Observation, Error> {
        validate_observation_field("id", id)?;
        self.conn
            .query_row(
                "SELECT id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts, scope
                   FROM observations WHERE id = ?1",
                [id],
                |row| {
                    let relevance: String = row.get(3)?;
                    let relevance = relevance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err))
                    })?;
                    let source_event_ids_json: String = row.get(4)?;
                    let source_event_ids = serde_json::from_str(&source_event_ids_json).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
                    })?;
                    let agent_kind: String = row.get(6)?;
                    let agent_kind = agent_kind.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err))
                    })?;
                    Ok(Observation {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        content: row.get(2)?,
                        relevance,
                        source_event_ids,
                        agent_id: row.get(5)?,
                        agent_kind,
                        derivation: row.get(7)?,
                        ts: row.get(8)?,
                        scope: row.get(9)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingObservation {
                    observation_id: id.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn list_observations_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<Observation>, Error> {
        validate_event_field("session_id", session_id)?;
        let pruned_observation_ids = self.pruned_observation_ids_for_session(session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM observations WHERE session_id = ?1 ORDER BY ts, id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut observations = Vec::new();
        for row in rows {
            let id = row?;
            if pruned_observation_ids.contains(&id) {
                continue;
            }
            observations.push(self.get_observation(&id)?);
        }
        Ok(observations)
    }

    pub fn observation_coverage(&self, session_id: &str) -> Result<ObservationCoverage, Error> {
        validate_event_field("session_id", session_id)?;
        let event_ids: Vec<i64> = self
            .list_events_for_session(session_id)?
            .into_iter()
            .map(|event| event.id)
            .collect();
        let covered: HashSet<i64> = self
            .list_observations_for_session(session_id)?
            .into_iter()
            .flat_map(|observation| observation.source_event_ids)
            .collect();
        let (covered_event_ids, uncovered_event_ids) = event_ids
            .iter()
            .copied()
            .partition(|event_id| covered.contains(event_id));

        Ok(ObservationCoverage {
            event_ids,
            covered_event_ids,
            uncovered_event_ids,
        })
    }

    pub fn recall_observation(&self, id: &str) -> Result<ObservationRecall, Error> {
        let observation = self.get_observation(id)?;
        let mut events = Vec::new();
        for event_id in &observation.source_event_ids {
            events.push(self.get_event(*event_id)?);
        }
        let prune_marker_id = self.prune_marker_id_for_observation(&observation.session_id, id)?;
        let audit_status = prune_marker_id.as_ref().map(|_| "pruned".to_string());
        Ok(ObservationRecall {
            observation,
            events,
            audit_status,
            prune_marker_id,
        })
    }

    pub(crate) fn pruned_observation_ids_for_session(
        &self,
        session_id: &str,
    ) -> Result<HashSet<String>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT pruned_observation_ids
               FROM observation_prune_markers
              WHERE session_id = ?1
           ORDER BY ts, id",
        )?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut pruned_observation_ids = HashSet::new();
        for row in rows {
            let ids_json = row?;
            let ids: Vec<String> = serde_json::from_str(&ids_json)?;
            pruned_observation_ids.extend(ids);
        }
        Ok(pruned_observation_ids)
    }

    pub(crate) fn prune_marker_id_for_observation(
        &self,
        session_id: &str,
        observation_id: &str,
    ) -> Result<Option<String>, Error> {
        validate_event_field("session_id", session_id)?;
        validate_observation_field("id", observation_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, pruned_observation_ids
               FROM observation_prune_markers
              WHERE session_id = ?1
           ORDER BY ts DESC, id DESC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (marker_id, ids_json) = row?;
            let ids: Vec<String> = serde_json::from_str(&ids_json)?;
            if ids.iter().any(|candidate| candidate == observation_id) {
                return Ok(Some(marker_id));
            }
        }
        Ok(None)
    }

    pub fn assemble_compaction_summary(&self, session_id: &str) -> Result<String, Error> {
        let observations = self.list_observations_for_session(session_id)?;
        let mut summary = String::from("# Aver session continuity summary\n\n");
        if observations.is_empty() {
            summary.push_str("No observations recorded.\n");
            return Ok(summary);
        }
        for observation in &observations {
            summary.push_str(&format!(
                "- [{}] {} (id={}, source_events={:?})\n",
                observation.relevance.as_str(),
                observation.content,
                observation.id,
                observation.source_event_ids
            ));
        }

        let coverage = self.observation_coverage(session_id)?;
        if !coverage.uncovered_event_ids.is_empty() {
            let mut ranges = Vec::new();
            let mut range_start = coverage.uncovered_event_ids[0];
            let mut range_end = coverage.uncovered_event_ids[0];

            for event_id in coverage.uncovered_event_ids.iter().skip(1).copied() {
                if event_id == range_end + 1 {
                    range_end = event_id;
                } else {
                    if range_start == range_end {
                        ranges.push(range_start.to_string());
                    } else {
                        ranges.push(format!("{range_start}-{range_end}"));
                    }
                    range_start = event_id;
                    range_end = event_id;
                }
            }
            if range_start == range_end {
                ranges.push(range_start.to_string());
            } else {
                ranges.push(format!("{range_start}-{range_end}"));
            }

            summary.push_str(&format!(
                "continuity is incomplete; uncovered event ranges: {}\n",
                ranges.join(", "),
            ));
        }

        Ok(summary)
    }

    pub fn prune_observations(&self, session_id: &str, keep: usize) -> Result<usize, Error> {
        validate_event_field("session_id", session_id)?;
        let mut observations = self.list_observations_for_session(session_id)?;
        if observations.len() <= keep {
            return Ok(0);
        }
        if !self
            .observation_coverage(session_id)?
            .uncovered_event_ids
            .is_empty()
        {
            return Ok(0);
        }
        observations.sort_by_key(|observation| {
            (
                observation.relevance.rank(),
                observation.ts,
                observation.id.clone(),
            )
        });
        let drop_count = observations.len() - keep;
        let pruned_observation_ids: Vec<String> = observations
            .iter()
            .take(drop_count)
            .map(|observation| observation.id.clone())
            .collect();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let marker_id = observation_prune_marker_id(session_id, &pruned_observation_ids, now);
        let entry = ObservationPruneLogEntry {
            kind: "prune_observations",
            ts: now,
            prune_marker_id: &marker_id,
            session_id,
            pruned_observation_ids: &pruned_observation_ids,
        };
        append_jsonl(&self.observation_log_path, &entry)?;
        self.conn.execute(
            "INSERT INTO observation_prune_markers
             (id, session_id, pruned_observation_ids, ts)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                marker_id,
                session_id,
                serde_json::to_string(&pruned_observation_ids)?,
                now,
            ],
        )?;
        Ok(drop_count)
    }

    pub fn graph_drift_snapshot(
        &self,
        consolidation: ConsolidationReport,
    ) -> Result<GraphDriftSnapshot, Error> {
        let mut claim_count_by_provenance = BTreeMap::new();
        let mut mean_confidence_by_provenance = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT provenance, COUNT(*), AVG(confidence)
               FROM claims GROUP BY provenance ORDER BY provenance",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        let mut total_claims = 0_u64;
        let mut ambiguous_claims = 0_u64;
        for row in rows {
            let (provenance, count, mean_confidence) = row?;
            if provenance == Provenance::Ambiguous.as_str() {
                ambiguous_claims = count;
            }
            total_claims += count;
            claim_count_by_provenance.insert(provenance.clone(), count);
            mean_confidence_by_provenance.insert(provenance, mean_confidence);
        }

        let contradicts_edge_count =
            self.conn
                .query_row("SELECT COUNT(*) FROM contradictions", [], |row| {
                    row.get::<_, u64>(0)
                })?;
        let mut entity_count_by_type_id = BTreeMap::new();
        let mut entity_stmt = self.conn.prepare(
            "SELECT entity_types.name, COUNT(entities.name)
               FROM entities JOIN entity_types ON entities.type_id = entity_types.id
              GROUP BY entity_types.name ORDER BY entity_types.name",
        )?;
        let entity_rows = entity_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in entity_rows {
            let (type_id, count) = row?;
            entity_count_by_type_id.insert(type_id, count);
        }

        let mut privacy_rejection_counts = BTreeMap::new();
        let mut privacy_stmt = self
            .conn
            .prepare("SELECT reason, count FROM privacy_rejections ORDER BY reason")?;
        let privacy_rows = privacy_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in privacy_rows {
            let (reason, count) = row?;
            privacy_rejection_counts.insert(reason, count);
        }

        Ok(GraphDriftSnapshot {
            claim_count_by_provenance,
            mean_confidence_by_provenance,
            contradicts_edge_count,
            ambiguous_ratio: if total_claims == 0 {
                0.0
            } else {
                ambiguous_claims as f64 / total_claims as f64
            },
            entity_count_by_type_id,
            consolidation_merged: consolidation.merged,
            consolidation_superseded: consolidation.superseded,
            privacy_rejection_counts,
        })
    }

    pub fn should_extract_memories(
        &self,
        session_id: &str,
        event_threshold: usize,
    ) -> Result<bool, Error> {
        // Coarse "enough accumulated to justify extraction" check: explicit
        // triggers and volume thresholds only. The coverage-gap reason is
        // surfaced by `extraction_decision` but does not flip this bool —
        // it is true for any session with undigested events, which would
        // make the event-count threshold meaningless.
        let decision = self.extraction_decision(session_id, event_threshold, None)?;
        Ok(decision
            .reasons
            .iter()
            .any(|reason| *reason != ExtractionTriggerReason::UncoveredCoverageGap))
    }

    pub fn extraction_decision(
        &self,
        session_id: &str,
        event_threshold: usize,
        observation_token_threshold: Option<usize>,
    ) -> Result<ExtractionDecision, Error> {
        validate_event_field("session_id", session_id)?;
        if event_threshold == 0 {
            return Err(Error::InvalidEventThreshold);
        }
        if observation_token_threshold == Some(0) {
            return Err(Error::InvalidEventThreshold);
        }

        let mut reasons = Vec::new();
        for (kind, reason) in [
            (
                "explicit_remember",
                ExtractionTriggerReason::ExplicitRemember,
            ),
            ("session_end", ExtractionTriggerReason::SessionEnd),
            ("correction", ExtractionTriggerReason::Correction),
            ("commit_completed", ExtractionTriggerReason::CommitCompleted),
            ("idle_compaction", ExtractionTriggerReason::IdleCompaction),
        ] {
            if self
                .conn
                .query_row(
                    "SELECT 1 FROM episodic_events
                      WHERE session_id = ?1 AND kind = ?2
                      LIMIT 1",
                    params![session_id, kind],
                    |_| Ok(()),
                )
                .is_ok()
            {
                reasons.push(reason);
            }
        }

        // Coverage gaps are a signal in their own right: they must surface
        // even when no observation-token threshold is configured.
        let coverage = self.observation_coverage(session_id)?;
        if !coverage.uncovered_event_ids.is_empty() {
            reasons.push(ExtractionTriggerReason::UncoveredCoverageGap);
        }

        let event_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM episodic_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        if event_count >= event_threshold {
            reasons.push(ExtractionTriggerReason::EventCountThreshold);
        }

        if let Some(threshold) = observation_token_threshold {
            let token_count: usize = self.conn.query_row(
                "SELECT COALESCE(SUM(
                    CASE
                      WHEN TRIM(payload) = '' THEN 0
                      ELSE LENGTH(TRIM(payload)) - LENGTH(REPLACE(TRIM(payload), ' ', '')) + 1
                    END), 0)
                   FROM episodic_events WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )?;
            if token_count >= threshold {
                reasons.push(ExtractionTriggerReason::ObservationTokenThreshold);
            }
        }

        Ok(ExtractionDecision {
            should_extract: !reasons.is_empty(),
            reasons,
        })
    }
}
