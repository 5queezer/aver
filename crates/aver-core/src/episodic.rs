//! Episodic events and extraction triggers.

use rusqlite::{params, types::Type};

use crate::apply::apply;
use crate::error::Error;
use crate::log::{LogRecord, append_jsonl};
use crate::store::Store;
use crate::types::{EpisodicEvent, EventWrite, ExtractionDecision, ExtractionTriggerReason};
use crate::validation::{validate_agent_id, validate_event_field, validate_scope};
impl Store {
    /// Record one episodic event (ADR-0014): validate → privacy gate → mint
    /// the id → log → apply, in one write transaction. Build the write with
    /// [`EventWrite::new`] plus `.agent(..)` / `.scope(..)`.
    pub fn record_event(&self, write: EventWrite) -> Result<i64, Error> {
        validate_event_field("session_id", &write.session_id)?;
        validate_event_field("kind", &write.kind)?;
        validate_event_field("source", &write.source)?;
        validate_agent_id(&write.agent_id)?;
        validate_scope(&write.scope)?;
        self.privacy_filter_recording(&format!(
            "{} {} {} {} {} {}",
            write.agent_id,
            write.agent_kind.as_str(),
            write.session_id,
            write.kind,
            write.payload,
            write.source
        ))?;
        self.privacy_filter_path_recording(&write.source)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // BEGIN IMMEDIATE serializes writers across processes (same race
        // class as insert_claim). Log append stays before the INSERT.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let event_id = self.ids.mint();
            let record = LogRecord::RecordEvent {
                ts: now,
                event_id,
                session_id: write.session_id,
                event_kind: write.kind,
                payload: write.payload,
                source: write.source,
                agent_id: write.agent_id,
                agent_kind: write.agent_kind.as_str().to_string(),
                scope: write.scope,
            };
            append_jsonl(&self.event_log_path, &record)?;
            apply(&self.conn, &record)?;
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
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope
               FROM episodic_events WHERE session_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let agent_kind: String = row.get(6)?;
            let agent_kind = agent_kind.parse().map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err))
            })?;
            Ok(EpisodicEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                kind: row.get(2)?,
                payload: row.get(3)?,
                source: row.get(4)?,
                agent_id: row.get(5)?,
                agent_kind,
                ts: row.get(7)?,
                scope: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn should_extract_memories(
        &self,
        session_id: &str,
        event_threshold: usize,
    ) -> Result<bool, Error> {
        // Coarse "enough accumulated to justify extraction" check: explicit
        // triggers and volume thresholds only.
        Ok(!self
            .extraction_decision(session_id, event_threshold)?
            .reasons
            .is_empty())
    }

    pub fn extraction_decision(
        &self,
        session_id: &str,
        event_threshold: usize,
    ) -> Result<ExtractionDecision, Error> {
        validate_event_field("session_id", session_id)?;
        if event_threshold == 0 {
            return Err(Error::InvalidEventThreshold);
        }
        let mut reasons = Vec::new();
        // The event `kind` that fires each reason is the reason's own name.
        for reason in [
            ExtractionTriggerReason::ExplicitRemember,
            ExtractionTriggerReason::SessionEnd,
            ExtractionTriggerReason::Correction,
            ExtractionTriggerReason::CommitCompleted,
        ] {
            let kind = reason.as_str();
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

        let event_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM episodic_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        if event_count >= event_threshold {
            reasons.push(ExtractionTriggerReason::EventCountThreshold);
        }

        Ok(ExtractionDecision {
            should_extract: !reasons.is_empty(),
            reasons,
        })
    }
}
