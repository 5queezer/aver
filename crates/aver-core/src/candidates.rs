//! Candidate claim staging, promotion, and rejection.

use std::str::FromStr;

use rusqlite::{OptionalExtension, params, types::Type};

use crate::error::Error;
use crate::log::{
    CandidateClaimLogEntry, LogEntry, PromoteCandidateLogEntry, RejectCandidateLogEntry,
    append_jsonl,
};
use crate::store::Store;
use crate::types::{CandidateClaim, CandidateClaimDraft, EpisodicEvent, Provenance};
use crate::validation::{
    validate_candidate_status_filter, validate_claim_field, validate_event_field,
    validate_rejection_reason, validate_scope,
};

pub trait ClaimExtractor {
    fn extract(&self, events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockClaimExtractor {
    drafts: Vec<CandidateClaimDraft>,
}

impl MockClaimExtractor {
    pub fn new(drafts: Vec<CandidateClaimDraft>) -> Self {
        Self { drafts }
    }
}

impl ClaimExtractor for MockClaimExtractor {
    fn extract(&self, _events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error> {
        Ok(self.drafts.clone())
    }
}

impl Store {
    pub fn propose_claims_from_extractor(
        &self,
        session_id: &str,
        extractor: &impl ClaimExtractor,
    ) -> Result<Vec<i64>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = extractor.extract(&events)?;
        let mut candidate_ids = Vec::new();
        for draft in drafts {
            candidate_ids.push(self.propose_candidate_claim(
                draft.event_id,
                &draft.subject,
                &draft.predicate,
                &draft.object,
            )?);
        }
        Ok(candidate_ids)
    }

    pub fn propose_candidate_claim(
        &self,
        event_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<i64, Error> {
        self.propose_candidate_claim_inner(event_id, subject, predicate, object, "global")
    }

    /// ADR-0021: variant of `propose_candidate_claim` that stages a candidate
    /// under an explicit scope. The scope rides through `promote_candidate_claim`
    /// onto the durable claim — there is no scope override at promotion time.
    pub fn propose_candidate_claim_with_scope(
        &self,
        event_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        self.propose_candidate_claim_inner(event_id, subject, predicate, object, scope)
    }

    pub(crate) fn propose_candidate_claim_inner(
        &self,
        event_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        if !self.event_exists(event_id)? {
            return Err(Error::MissingEventProvenance { event_id });
        }
        validate_claim_field("subject", subject)?;
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("object", object)?;
        self.privacy_filter_recording(&format!("{subject} {predicate} {object}"))?;
        for possible_path in [subject, object] {
            self.privacy_filter_path_recording(possible_path)?;
        }
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // The candidate id is pre-allocated inside a write transaction so the
        // append-only log can pin it (ADR-0005) and cross-process writers
        // cannot allocate the same id. Candidate records go to the episodic
        // log: they FK-reference episodic_events, which replay in the same
        // phase (ADR-0019 §4 input order replays events.jsonl after
        // log.jsonl, so log.jsonl must not carry event-dependent records).
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let candidate_id: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM candidate_claims",
                [],
                |r| r.get(0),
            )?;
            let entry = CandidateClaimLogEntry {
                kind: "propose_candidate_claim",
                ts: now,
                candidate_id,
                event_id,
                subject,
                predicate,
                object,
                scope,
            };
            append_jsonl(&self.event_log_path, &entry)?;
            self.conn.execute(
                "INSERT INTO candidate_claims (id, event_id, subject, predicate, object, created_at, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![candidate_id, event_id, subject, predicate, object, now, scope],
            )?;
            Ok(candidate_id)
        })();
        match result {
            Ok(candidate_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(candidate_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub(crate) fn event_exists(&self, event_id: i64) -> Result<bool, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM episodic_events WHERE id = ?1",
                [event_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn promote_candidate_claim(&self, candidate_id: i64) -> Result<i64, Error> {
        let candidate = self.get_candidate_claim(candidate_id)?;
        if candidate.status == "REJECTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        if let Some(claim_id) = candidate.promoted_claim_id {
            return Ok(claim_id);
        }
        let event = self.get_event(candidate.event_id)?;
        let source = format!("event:{}", event.id);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Validate before log append; apply any USER_ASSERTED ontology extension
        // only after the append boundary inside the write transaction.
        let extend_ontology = self.validate_ontology(&candidate.predicate, candidate.provenance)?;

        // Id allocation, log appends, and projection updates in one write
        // transaction (same race class as insert_claim). Log-first ordering
        // is preserved: both log lines precede the SQLite writes.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            // Re-read under the write lock so racing promoters cannot both
            // observe PENDING and create separate claims for one candidate.
            let current: Option<(String, Option<i64>)> = self
                .conn
                .query_row(
                    "SELECT status, promoted_claim_id FROM candidate_claims WHERE id = ?1",
                    [candidate_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((status, promoted_claim_id)) = current else {
                return Err(Error::MissingCandidate { candidate_id });
            };
            if let Some(claim_id) = promoted_claim_id {
                return Ok(claim_id);
            }
            if status != "PENDING" {
                return Err(Error::InvalidCandidateStatus {
                    candidate_id,
                    status,
                });
            }

            let claim_id: i64 =
                self.conn
                    .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                        r.get(0)
                    })?;

            let entry = LogEntry {
                kind: "add_claim",
                ts: now,
                claim_id,
                subject: &candidate.subject,
                predicate: &candidate.predicate,
                object: &candidate.object,
                source: &source,
                agent_id: &event.agent_id,
                agent_kind: event.agent_kind.as_str(),
                confidence: candidate.confidence,
                provenance: candidate.provenance.as_str(),
                scope: &candidate.scope,
            };
            append_jsonl(&self.log_path, &entry)?;
            append_jsonl(&self.agent_log_path(&event.agent_id)?, &entry)?;
            // The promotion marker joins the candidate in the episodic log
            // (FK dependency on candidate_claims, which replays in the
            // events phase; the paired add_claim line replays earlier).
            let promote_entry = PromoteCandidateLogEntry {
                kind: "promote_candidate_claim",
                ts: now,
                candidate_id,
                claim_id,
            };
            append_jsonl(&self.event_log_path, &promote_entry)?;

            if extend_ontology {
                self.apply_ontology_extension(&candidate.predicate, &event.agent_id, now)?;
            }

            self.ensure_entity(&candidate.subject, now)?;
            self.ensure_entity(&candidate.object, now)?;

            let source_refs = serde_json::to_string(&[source])?;
            // last_verified_at starts at creation, same as add_claim and as
            // replay's apply_add_claim (which only sees the add_claim line).
            self.conn.execute(
                "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                     status, source_refs, agent_id, agent_kind, write_ts,
                                     created_at, last_seen_at, last_verified_at, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7, ?8, ?9, ?10, ?10, ?10, ?10, ?11)",
                params![
                    claim_id,
                    candidate.subject,
                    candidate.predicate,
                    candidate.object,
                    candidate.provenance.as_str(),
                    candidate.confidence,
                    source_refs,
                    event.agent_id,
                    event.agent_kind.as_str(),
                    now,
                    candidate.scope,
                ],
            )?;
            self.conn.execute(
                "UPDATE candidate_claims
                    SET status = 'PROMOTED', promoted_claim_id = ?1
                  WHERE id = ?2",
                params![claim_id, candidate_id],
            )?;
            Ok(claim_id)
        })();
        match result {
            Ok(claim_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(claim_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn reject_candidate_claim(&self, candidate_id: i64, reason: &str) -> Result<(), Error> {
        let candidate = self.get_candidate_claim(candidate_id)?;
        if candidate.status == "PROMOTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        validate_rejection_reason(reason)?;
        self.privacy_filter_recording(reason)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<(), Error> {
            // Re-read under the write lock so racing callers cannot both
            // observe PENDING and append conflicting terminal transitions.
            let current: Option<(String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT status, rejection_reason FROM candidate_claims WHERE id = ?1",
                    [candidate_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((status, rejection_reason)) = current else {
                return Err(Error::MissingCandidate { candidate_id });
            };
            if status == "REJECTED" {
                if rejection_reason.as_deref() == Some(reason) {
                    return Ok(());
                }
                return Err(Error::InvalidCandidateStatus {
                    candidate_id,
                    status,
                });
            }
            if status != "PENDING" {
                return Err(Error::InvalidCandidateStatus {
                    candidate_id,
                    status,
                });
            }

            // Candidate lifecycle records live in the episodic log so replay
            // applies them in the same phase as the candidate proposal.
            let entry = RejectCandidateLogEntry {
                kind: "reject_candidate_claim",
                ts: now,
                candidate_id,
                reason,
            };
            append_jsonl(&self.event_log_path, &entry)?;
            let rows_changed = self.conn.execute(
                "UPDATE candidate_claims
                    SET status = 'REJECTED', rejection_reason = ?1
                  WHERE id = ?2",
                params![reason, candidate_id],
            )?;
            if rows_changed == 0 {
                return Err(Error::MissingCandidate { candidate_id });
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn list_candidate_claims(
        &self,
        session_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<CandidateClaim>, Error> {
        if let Some(session_id) = session_id {
            validate_event_field("session_id", session_id)?;
        }
        if let Some(status) = status {
            validate_candidate_status_filter(status)?;
        }
        const CANDIDATE_COLUMNS: &str = "candidate_claims.id, candidate_claims.event_id,
            candidate_claims.subject, candidate_claims.predicate, candidate_claims.object,
            candidate_claims.provenance, candidate_claims.confidence, candidate_claims.status,
            candidate_claims.promoted_claim_id, candidate_claims.rejection_reason,
            candidate_claims.scope";
        let (sql, bind_status, bind_session): (String, Option<&str>, Option<&str>) = match (
            status, session_id,
        ) {
            (Some(status), Some(session_id)) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS}
                           FROM candidate_claims
                           JOIN episodic_events ON episodic_events.id = candidate_claims.event_id
                          WHERE candidate_claims.status = ?1 AND episodic_events.session_id = ?2
                          ORDER BY candidate_claims.id"
                ),
                Some(status),
                Some(session_id),
            ),
            (Some(status), None) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS} FROM candidate_claims WHERE status = ?1 ORDER BY id"
                ),
                Some(status),
                None,
            ),
            (None, Some(session_id)) => (
                format!(
                    "SELECT {CANDIDATE_COLUMNS}
                           FROM candidate_claims
                           JOIN episodic_events ON episodic_events.id = candidate_claims.event_id
                          WHERE episodic_events.session_id = ?1
                          ORDER BY candidate_claims.id"
                ),
                None,
                Some(session_id),
            ),
            (None, None) => (
                format!("SELECT {CANDIDATE_COLUMNS} FROM candidate_claims ORDER BY id"),
                None,
                None,
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let map_candidate = |row: &rusqlite::Row<'_>| {
            let provenance_str = row.get::<usize, String>(5)?;
            let provenance = Provenance::from_str(&provenance_str).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
            })?;
            Ok(CandidateClaim {
                id: row.get(0)?,
                event_id: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                provenance,
                confidence: row.get(6)?,
                status: row.get(7)?,
                promoted_claim_id: row.get(8)?,
                rejection_reason: row.get(9)?,
                scope: row.get(10)?,
            })
        };
        let rows = match (bind_status, bind_session) {
            (Some(status), Some(session_id)) => stmt
                .query_map(params![status, session_id], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(status), None) => stmt
                .query_map([status], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(session_id)) => stmt
                .query_map([session_id], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([], map_candidate)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    /// List candidate claims for a specific event.
    pub fn list_candidate_claims_for_event(
        &self,
        event_id: i64,
    ) -> Result<Vec<CandidateClaim>, Error> {
        if !self.event_exists(event_id)? {
            return Err(Error::MissingEventProvenance { event_id });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, subject, predicate, object, provenance, confidence, status, promoted_claim_id, rejection_reason, scope
             FROM candidate_claims WHERE event_id = ?1",
        )?;
        let rows = stmt.query_map([event_id], |row| {
            let provenance_str = row.get::<usize, String>(5)?;
            let provenance = Provenance::from_str(&provenance_str).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
            })?;
            Ok(CandidateClaim {
                id: row.get(0)?,
                event_id: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                provenance,
                confidence: row.get(6)?,
                status: row.get(7)?,
                promoted_claim_id: row.get(8)?,
                rejection_reason: row.get(9)?,
                scope: row.get(10)?,
            })
        })?;
        let mut candidates = Vec::new();
        for row in rows {
            candidates.push(row?);
        }
        Ok(candidates)
    }

    pub fn get_candidate_claim(&self, id: i64) -> Result<CandidateClaim, Error> {
        self.conn
            .query_row(
                "SELECT id, event_id, subject, predicate, object, provenance, confidence, status,
                    promoted_claim_id, rejection_reason, scope
               FROM candidate_claims WHERE id = ?1",
                [id],
                |row| {
                    let provenance: String = row.get(5)?;
                    let provenance = provenance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
                    })?;
                    Ok(CandidateClaim {
                        id: row.get(0)?,
                        event_id: row.get(1)?,
                        subject: row.get(2)?,
                        predicate: row.get(3)?,
                        object: row.get(4)?,
                        provenance,
                        confidence: row.get(6)?,
                        status: row.get(7)?,
                        promoted_claim_id: row.get(8)?,
                        rejection_reason: row.get(9)?,
                        scope: row.get(10)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::MissingCandidate { candidate_id: id }
                }
                other => Error::Sqlite(other),
            })
    }
}
