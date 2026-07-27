//! Claim write/get/recall/consolidate/retire and privacy-recording helpers.

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::error::Error;
use crate::log::{
    ConfidenceChange, ContradictionLogEntry, DecayLogEntry, LogEntry, MergeSourceRefsLogEntry,
    RetireClaimLogEntry, SupersedeLogEntry, append_jsonl,
};
use crate::privacy::{PrivacyRejection, privacy_filter, privacy_filter_path};
use crate::recall::{query_tokens_for_recall, recall_token_score};
use crate::store::{Store, scope_filter_sql};
use crate::types::{
    AgentKind, Claim, ConsolidationReport, ContradictionRecord, ExtractorFact, NewClaim,
    PredicateWalk, Provenance, RecallFilters, ScopeWalk,
};
use crate::validation::{
    validate_agent_id, validate_claim_field, validate_contradiction_reason, validate_scope,
};

pub(crate) type ClaimRow = (
    i64,
    String,
    String,
    String,
    String,
    f64,
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    String, // scope (ADR-0021)
);

pub(crate) struct ClaimWrite<'a> {
    agent_id: &'a str,
    agent_kind: AgentKind,
    provenance: Provenance,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    source: &'a str,
    confidence: f64,
    /// ADR-0021 memory scope. `"global"` is the safe default that reproduces
    /// pre-scope behavior verbatim.
    scope: &'a str,
}

pub(crate) fn provenance_for_agent_kind(agent_kind: AgentKind) -> Provenance {
    match agent_kind {
        AgentKind::Human => Provenance::UserAsserted,
        AgentKind::DeterministicParser | AgentKind::ExternalTool => Provenance::Extracted,
        AgentKind::Llm => Provenance::Inferred,
    }
}

impl Store {
    /// Append a USER_ASSERTED claim. Pre-allocates the claim id, writes
    /// the JSONL log line first (source of truth, ADR-0005), then mirrors
    /// into SQLite with the same explicit id so audit replay can rebuild
    /// the DB from the log without id drift.
    /// Default confidence is 0.95 per ADR-0003's policy table.
    pub fn add_claim(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.add_claim_with_confidence(subject, predicate, object, source, 0.95)
    }

    pub fn add_claim_with_confidence(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        confidence: f64,
    ) -> Result<i64, Error> {
        self.insert_claim(ClaimWrite {
            agent_id: "local",
            agent_kind: AgentKind::Human,
            provenance: Provenance::UserAsserted,
            subject,
            predicate,
            object,
            source,
            confidence,
            scope: "global",
        })
    }

    pub fn add_claim_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        let provenance = provenance_for_agent_kind(agent_kind);
        self.insert_claim(ClaimWrite {
            agent_id,
            agent_kind,
            provenance,
            subject,
            predicate,
            object,
            source,
            confidence: provenance.policy_confidence(),
            scope: "global",
        })
    }

    /// ADR-0021: variant of `add_claim` that records under an explicit scope.
    pub fn add_claim_with_scope(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        self.insert_claim(ClaimWrite {
            agent_id: "local",
            agent_kind: AgentKind::Human,
            provenance: Provenance::UserAsserted,
            subject,
            predicate,
            object,
            source,
            confidence: 0.95,
            scope,
        })
    }

    /// ADR-0021: variant of `add_claim_with_confidence` that records under an
    /// explicit scope.
    pub fn add_claim_with_confidence_and_scope(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        confidence: f64,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        self.insert_claim(ClaimWrite {
            agent_id: "local",
            agent_kind: AgentKind::Human,
            provenance: Provenance::UserAsserted,
            subject,
            predicate,
            object,
            source,
            confidence,
            scope,
        })
    }

    /// ADR-0021: variant of `add_claim_from_agent` that records under an
    /// explicit scope.
    #[allow(clippy::too_many_arguments)]
    pub fn add_claim_from_agent_with_scope(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        scope: &str,
    ) -> Result<i64, Error> {
        validate_scope(scope)?;
        let provenance = provenance_for_agent_kind(agent_kind);
        self.insert_claim(ClaimWrite {
            agent_id,
            agent_kind,
            provenance,
            subject,
            predicate,
            object,
            source,
            confidence: provenance.policy_confidence(),
            scope,
        })
    }

    pub fn ingest_extractor_facts(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        source: &str,
        facts: &[ExtractorFact],
    ) -> Result<Vec<i64>, Error> {
        let writes = facts
            .iter()
            .map(|fact| {
                let provenance = fact
                    .provenance
                    .unwrap_or_else(|| provenance_for_agent_kind(agent_kind));
                ClaimWrite {
                    agent_id,
                    agent_kind,
                    provenance,
                    subject: &fact.subject,
                    predicate: &fact.predicate,
                    object: &fact.object,
                    source,
                    confidence: provenance.policy_confidence(),
                    scope: "global",
                }
            })
            .collect::<Vec<_>>();
        // Fail the batch before any per-claim log append/commit. insert_claim
        // deliberately revalidates each write at its own append boundary.
        for write in &writes {
            self.validate_claim_write(write)?;
        }

        let mut ids = Vec::with_capacity(writes.len());
        for write in writes {
            let id = self.insert_claim(write)?;
            ids.push(id);
        }
        self.consolidate_report()?;
        Ok(ids)
    }

    pub(crate) fn validate_claim_write(&self, write: &ClaimWrite<'_>) -> Result<(), Error> {
        validate_claim_field("subject", write.subject)?;
        validate_claim_field("predicate", write.predicate)?;
        validate_claim_field("object", write.object)?;
        validate_claim_field("source", write.source)?;
        if !(0.0..=1.0).contains(&write.confidence) {
            return Err(Error::InvalidConfidence {
                value: write.confidence,
            });
        }
        validate_agent_id(write.agent_id)?;
        if let Err(rejection) = privacy_filter(&format!(
            "{} {} {} {} {} {}",
            write.agent_id,
            write.agent_kind.as_str(),
            write.subject,
            write.predicate,
            write.object,
            write.source
        )) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        for possible_path in [write.subject, write.object, write.source] {
            self.privacy_filter_path_recording(possible_path)?;
        }

        self.validate_ontology(write.predicate, write.provenance)
            .map(|_| ())
    }

    pub(crate) fn insert_claim(&self, write: ClaimWrite<'_>) -> Result<i64, Error> {
        self.validate_claim_write(&write)?;
        let extend_ontology = self.validate_ontology(write.predicate, write.provenance)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Pre-allocate the claim id inside a write transaction. The
        // BEGIN IMMEDIATE serializes cross-process writers (WAL single
        // writer), so two processes can no longer compute the same
        // MAX(id)+1 and poison the log with duplicate claim ids. The log
        // append stays strictly before the SQLite INSERT (ADR-0005).
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let claim_id: i64 =
                self.conn
                    .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                        r.get(0)
                    })?;

            let entry = LogEntry {
                kind: "add_claim",
                ts: now,
                claim_id,
                subject: write.subject,
                predicate: write.predicate,
                object: write.object,
                source: write.source,
                agent_id: write.agent_id,
                agent_kind: write.agent_kind.as_str(),
                confidence: write.confidence,
                provenance: write.provenance.as_str(),
                scope: write.scope,
            };
            append_jsonl(&self.log_path, &entry)?;
            append_jsonl(&self.agent_log_path(write.agent_id)?, &entry)?;

            if extend_ontology {
                self.apply_ontology_extension(write.predicate, write.agent_id, now)?;
            }

            self.ensure_entity(write.subject, now)?;
            self.ensure_entity(write.object, now)?;

            let source_refs = serde_json::to_string(&[write.source])?;
            self.conn.execute(
                "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                     status, source_refs, agent_id, agent_kind, write_ts,
                                     created_at, last_seen_at, last_verified_at, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7,
                         ?8, ?9, ?10, ?10, ?10, ?10, ?11)",
                params![
                    claim_id,
                    write.subject,
                    write.predicate,
                    write.object,
                    write.provenance.as_str(),
                    write.confidence,
                    source_refs,
                    write.agent_id,
                    write.agent_kind.as_str(),
                    now,
                    write.scope,
                ],
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

    pub(crate) fn record_privacy_rejection(
        &self,
        rejection: PrivacyRejection,
    ) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO privacy_rejections (reason, count) VALUES (?1, 1)
             ON CONFLICT(reason) DO UPDATE SET count = count + 1",
            [rejection.telemetry_reason()],
        )?;
        Ok(())
    }

    pub(crate) fn privacy_filter_recording(&self, content: &str) -> Result<(), Error> {
        if let Err(rejection) = privacy_filter(content) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        Ok(())
    }

    pub fn privacy_filter_path_recording(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        if let Err(rejection) = privacy_filter_path(path) {
            self.record_privacy_rejection(rejection)?;
            return Err(Error::Privacy(rejection));
        }
        Ok(())
    }

    pub fn privacy_rejection_count(&self, rejection: PrivacyRejection) -> Result<i64, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT count FROM privacy_rejections WHERE reason = ?1",
                [rejection.telemetry_reason()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// Add a contradiction record for a claim.
    pub fn add_contradiction(&self, claim_id: i64, reason: &str) -> Result<i64, Error> {
        validate_claim_field("reason", reason)?;
        self.privacy_filter_recording(reason)?;
        self.ensure_claim_exists(claim_id)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // The contradiction id is pre-allocated inside a write transaction so
        // the log record pins the same id the projection uses (ADR-0005).
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let contradiction_id: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM contradictions",
                [],
                |r| r.get(0),
            )?;
            let entry = ContradictionLogEntry {
                kind: "add_contradiction",
                ts: now,
                contradiction_id,
                claim_id,
                reason,
                new_claim_id: None,
            };
            append_jsonl(&self.log_path, &entry)?;
            self.conn.execute(
                "INSERT INTO contradictions (id, claim_id, reason, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![contradiction_id, claim_id, reason, now],
            )?;
            Ok(contradiction_id)
        })();
        match result {
            Ok(contradiction_id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(contradiction_id)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Retrieve a claim by id.
    pub fn get_claim(&self, id: i64) -> Result<Claim, Error> {
        let (
            id,
            subject,
            predicate,
            object,
            provenance,
            confidence,
            status,
            source_refs,
            agent_id,
            agent_kind,
            write_ts,
            last_verified_at,
            scope,
        ): ClaimRow = self
            .conn
            .query_row(
                "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts, last_verified_at, scope
               FROM claims WHERE id = ?1",
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
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                    ))
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingClaim { claim_id: id },
                other => Error::Sqlite(other),
            })?;

        Ok(Claim {
            id,
            subject,
            predicate,
            object,
            provenance: provenance.parse()?,
            confidence,
            status: status.parse()?,
            source_refs: serde_json::from_str(&source_refs)?,
            agent_id,
            agent_kind: agent_kind.parse()?,
            write_ts,
            last_verified_at,
            scope,
        })
    }

    pub fn agent_trust_score(&self, agent_id: &str) -> Result<f64, Error> {
        validate_agent_id(agent_id)?;
        let (active, total): (i64, i64) = self.conn.query_row(
            "SELECT
                 SUM(CASE WHEN status = 'ACTIVE' THEN 1 ELSE 0 END),
                 COUNT(*)
               FROM claims
              WHERE agent_id = ?1",
            [agent_id],
            |row| Ok((row.get::<_, Option<i64>>(0)?.unwrap_or(0), row.get(1)?)),
        )?;
        if total == 0 {
            return Ok(0.5);
        }
        Ok(((active as f64) / (total as f64)).clamp(0.1, 1.0))
    }

    pub fn contradict(
        &self,
        claim_id: i64,
        reason: &str,
        new_claim: Option<NewClaim<'_>>,
    ) -> Result<ContradictionRecord, Error> {
        self.ensure_claim_exists(claim_id)?;
        validate_contradiction_reason(reason)?;
        self.privacy_filter_recording(reason)?;
        let new_claim_id = if let Some(claim) = new_claim {
            // This is an intentional transaction boundary: add_claim commits
            // and logs independently. A later contradiction failure therefore
            // leaves the replacement claim durable; retrying may create another.
            Some(self.add_claim(claim.subject, claim.predicate, claim.object, claim.source)?)
        } else {
            None
        };
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // Pre-allocate the contradiction id inside a write transaction so the
        // log record pins the same id the projection uses (ADR-0005).
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<i64, Error> {
            let contradiction_id: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM contradictions",
                [],
                |r| r.get(0),
            )?;
            let entry = ContradictionLogEntry {
                kind: "add_contradiction",
                ts: now,
                contradiction_id,
                claim_id,
                reason,
                new_claim_id,
            };
            append_jsonl(&self.log_path, &entry)?;
            self.conn.execute(
                "INSERT INTO contradictions (id, claim_id, reason, new_claim_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![contradiction_id, claim_id, reason, new_claim_id, now],
            )?;
            Ok(contradiction_id)
        })();
        let id = match result {
            Ok(id) => {
                self.conn.execute_batch("COMMIT")?;
                id
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                return Err(err);
            }
        };
        self.get_contradiction(id)
    }

    pub fn list_contradictions(&self, claim_id: i64) -> Result<Vec<ContradictionRecord>, Error> {
        self.ensure_claim_exists(claim_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM contradictions
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([claim_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.get_contradiction(id))
            .collect()
    }

    pub(crate) fn get_contradiction(&self, id: i64) -> Result<ContradictionRecord, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, reason, new_claim_id, status, created_at
               FROM contradictions
              WHERE id = ?1",
                [id],
                |row| {
                    Ok(ContradictionRecord {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        reason: row.get(2)?,
                        new_claim_id: row.get(3)?,
                        status: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    pub fn should_merge_synonym(similarity: f64) -> bool {
        similarity >= 0.92
    }

    pub fn decay_contradicted_confidence(&self) -> Result<usize, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // The concrete post-decay values are computed up front and logged
        // before the projection UPDATEs (ADR-0005): the log records outcomes,
        // not the decay formula, so replay reproduces the same state even if
        // the decay policy changes later.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<usize, Error> {
            let mut stmt = self.conn.prepare(
                "SELECT id, confidence
                   FROM claims
                  WHERE status = 'ACTIVE'
                    AND EXISTS (
                        SELECT 1
                          FROM contradictions
                         WHERE contradictions.claim_id = claims.id
                           AND contradictions.status = 'RECORDED'
                    )
                  ORDER BY id",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            // Mirrors SQL MAX(0.0, ROUND(confidence - 0.10, 2)).
            let changes: Vec<ConfidenceChange> = rows
                .into_iter()
                .map(|(claim_id, confidence)| {
                    let decayed = ((confidence - 0.10) * 100.0).round() / 100.0;
                    ConfidenceChange {
                        claim_id,
                        confidence: decayed.max(0.0),
                    }
                })
                .collect();
            if changes.is_empty() {
                return Ok(0);
            }
            let entry = DecayLogEntry {
                kind: "decay_confidence",
                ts: now,
                changes: &changes,
            };
            append_jsonl(&self.log_path, &entry)?;
            for change in &changes {
                self.conn.execute(
                    "UPDATE claims SET confidence = ?1 WHERE id = ?2",
                    params![change.confidence, change.claim_id],
                )?;
            }
            Ok(changes.len())
        })();
        match result {
            Ok(changed) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(changed)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn decay_inferred_confidence_at(
        &self,
        now_ts: i64,
        tau_seconds: f64,
    ) -> Result<usize, Error> {
        if tau_seconds <= 0.0 || !tau_seconds.is_finite() {
            return Err(Error::InvalidDecayTau { value: tau_seconds });
        }
        let log_ts = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<usize, Error> {
            let mut stmt = self.conn.prepare(
                "SELECT id, confidence, last_seen_at
                   FROM claims
                  WHERE status = 'ACTIVE' AND provenance = 'INFERRED'
                  ORDER BY id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut changes = Vec::with_capacity(rows.len());
            for (id, confidence, last_seen_at) in rows {
                let delta = now_ts.saturating_sub(last_seen_at).max(0) as f64;
                let decayed = confidence * (-delta / tau_seconds).exp();
                changes.push(ConfidenceChange {
                    claim_id: id,
                    confidence: decayed,
                });
            }
            if changes.is_empty() {
                return Ok(0);
            }
            let entry = DecayLogEntry {
                kind: "decay_confidence",
                ts: log_ts,
                changes: &changes,
            };
            append_jsonl(&self.log_path, &entry)?;
            for change in &changes {
                self.conn.execute(
                    "UPDATE claims SET confidence = ?1 WHERE id = ?2",
                    params![change.confidence, change.claim_id],
                )?;
            }
            Ok(changes.len())
        })();
        match result {
            Ok(changed) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(changed)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn consolidate(&self) -> Result<usize, Error> {
        Ok(self.consolidate_report()?.superseded)
    }

    pub fn consolidate_report(&self) -> Result<ConsolidationReport, Error> {
        let merged = self.merge_duplicate_source_refs()?;
        let decayed = self.decay_contradicted_confidence()?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // Supersede outcomes are captured as explicit id lists and logged
        // before the projection UPDATEs (ADR-0005), so replay asserts the
        // same lifecycle transitions instead of recomputing them.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<usize, Error> {
            let duplicate_ids = self.claim_ids_matching(
                "SELECT id
                   FROM claims
                  WHERE id NOT IN (
                        SELECT MIN(id)
                          FROM claims
                         GROUP BY subject, predicate, object
                    )
                    AND status = 'ACTIVE'
                  ORDER BY id",
            )?;
            let duplicate_changed = self.supersede_claim_ids(&duplicate_ids, now)?;
            let conflict_ids = self.claim_ids_matching(
                "SELECT id
                   FROM claims
                  WHERE status = 'ACTIVE'
                    AND EXISTS (
                        SELECT 1
                          FROM claims newer
                         WHERE newer.subject = claims.subject
                           AND newer.predicate = claims.predicate
                           AND newer.object <> claims.object
                           AND newer.id > claims.id
                    )
                  ORDER BY id",
            )?;
            let conflict_changed = self.supersede_claim_ids(&conflict_ids, now)?;
            Ok(duplicate_changed + conflict_changed)
        })();
        match result {
            Ok(superseded) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(ConsolidationReport {
                    merged,
                    superseded,
                    decayed,
                })
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub(crate) fn claim_ids_matching(&self, sql: &str) -> Result<Vec<i64>, Error> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Log a `supersede_claims` record and flip the given claims to
    /// SUPERSEDED. Caller must hold a write transaction. Empty id lists are
    /// neither logged nor updated.
    pub(crate) fn supersede_claim_ids(&self, claim_ids: &[i64], now: i64) -> Result<usize, Error> {
        if claim_ids.is_empty() {
            return Ok(0);
        }
        let entry = SupersedeLogEntry {
            kind: "supersede_claims",
            ts: now,
            claim_ids,
        };
        append_jsonl(&self.log_path, &entry)?;
        for claim_id in claim_ids {
            self.conn.execute(
                "UPDATE claims SET status = 'SUPERSEDED' WHERE id = ?1",
                [claim_id],
            )?;
        }
        Ok(claim_ids.len())
    }

    pub(crate) fn merge_duplicate_source_refs(&self) -> Result<usize, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT subject, predicate, object, MIN(id)
               FROM claims
              GROUP BY subject, predicate, object
             HAVING COUNT(*) > 1",
        )?;
        let groups = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        if groups.is_empty() {
            return Ok(0);
        }

        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<usize, Error> {
            let mut merged_groups = 0;
            for (subject, predicate, object, survivor_id) in groups {
                let mut source_refs = Vec::new();
                let mut refs_stmt = self.conn.prepare(
                    "SELECT source_refs
                       FROM claims
                      WHERE subject = ?1 AND predicate = ?2 AND object = ?3
                      ORDER BY id",
                )?;
                let refs_rows = refs_stmt
                    .query_map(params![subject, predicate, object], |row| {
                        row.get::<_, String>(0)
                    })?;
                for refs_json in refs_rows {
                    for source_ref in serde_json::from_str::<Vec<String>>(&refs_json?)? {
                        if !source_refs.contains(&source_ref) {
                            source_refs.push(source_ref);
                        }
                    }
                }
                let merged = serde_json::to_string(&source_refs)?;
                let survivor = self.get_claim(survivor_id)?;
                let should_promote =
                    survivor.provenance == Provenance::Inferred && source_refs.len() >= 2;
                let entry = MergeSourceRefsLogEntry {
                    kind: "merge_source_refs",
                    ts: now,
                    claim_id: survivor_id,
                    source_refs: &source_refs,
                    promote: should_promote,
                };
                append_jsonl(&self.log_path, &entry)?;
                if should_promote {
                    self.conn.execute(
                        "UPDATE claims
                            SET source_refs = ?1, provenance = 'EXTRACTED', confidence = MAX(confidence, 0.75)
                          WHERE id = ?2",
                        params![merged, survivor_id],
                    )?;
                } else {
                    self.conn.execute(
                        "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
                        params![merged, survivor_id],
                    )?;
                }
                merged_groups += 1;
            }
            Ok(merged_groups)
        })();
        match result {
            Ok(merged) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(merged)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn recall_text(&self, query: &str) -> Result<Vec<Claim>, Error> {
        self.recall_text_inner(query, "global", ScopeWalk::Any)
    }

    /// ADR-0021: variant of `recall_text` that filters by `scope` per `walk`.
    /// `scope_walk = Any` ignores the scope argument and reproduces today's
    /// global-search behavior.
    pub fn recall_text_with_scope(
        &self,
        query: &str,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<Vec<Claim>, Error> {
        validate_scope(scope)?;
        self.recall_text_inner(query, scope, walk)
    }

    pub(crate) fn recall_text_inner(
        &self,
        query: &str,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<Vec<Claim>, Error> {
        let filters = RecallFilters {
            scope: scope.to_string(),
            scope_walk: walk,
            ..RecallFilters::default()
        };
        self.recall_text_with_filters(query, filters)
    }

    /// ADR-0023: typed-filter recall. Combines scope (ADR-0021) with
    /// agent_id, agent_kind, predicate (with closure walk), min_confidence,
    /// and status filters. `RecallFilters::default()` reproduces today's
    /// behavior verbatim.
    pub fn recall_text_with_filters(
        &self,
        query: &str,
        filters: RecallFilters,
    ) -> Result<Vec<Claim>, Error> {
        validate_scope(&filters.scope)?;
        if let Some(min) = filters.min_confidence
            && !(0.0..=1.0).contains(&min)
        {
            return Err(Error::InvalidConfidence { value: min });
        }
        let query_tokens = query_tokens_for_recall(query);
        if query_tokens.is_empty() {
            return Err(Error::InvalidRecallQuery);
        }

        let mut where_parts: Vec<String> = Vec::new();
        let mut query_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        // Build each clause and its bind together so placeholder order remains
        // structural as filters evolve.
        if let Some(status) = filters.status {
            where_parts.push("status = ?".to_string());
            query_params.push(Box::new(status.as_str().to_string()));
        }
        let (scope_clause, scope_params) = scope_filter_sql(&filters.scope, filters.scope_walk);
        where_parts.push(scope_clause);
        query_params.extend(
            scope_params
                .into_iter()
                .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
        );
        if let Some(agent_id) = &filters.agent_id {
            where_parts.push("agent_id = ?".to_string());
            query_params.push(Box::new(agent_id.clone()));
        }
        if let Some(agent_kind) = filters.agent_kind {
            where_parts.push("agent_kind = ?".to_string());
            query_params.push(Box::new(agent_kind.as_str().to_string()));
        }
        if let Some(predicate) = &filters.predicate {
            match filters.predicate_walk {
                PredicateWalk::Exact => {
                    where_parts.push("predicate = ?".to_string());
                    query_params.push(Box::new(predicate.clone()));
                }
                PredicateWalk::Descendants => {
                    let allowed = self.expand_predicate_filter(&[predicate.as_str()])?;
                    if allowed.is_empty() {
                        return Ok(Vec::new());
                    }
                    let mut allowed: Vec<String> = allowed.into_iter().collect();
                    allowed.sort();
                    let placeholders = std::iter::repeat_n("?", allowed.len())
                        .collect::<Vec<_>>()
                        .join(",");
                    where_parts.push(format!("predicate IN ({placeholders})"));
                    query_params.extend(
                        allowed
                            .into_iter()
                            .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
                    );
                }
            }
        }
        if let Some(confidence) = filters.min_confidence {
            where_parts.push("confidence >= ?".to_string());
            query_params.push(Box::new(confidence));
        }
        let where_sql = where_parts.join(" AND ");

        let sql = format!(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts, last_verified_at, scope
               FROM claims
              WHERE {where_sql}
              ORDER BY id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(query_params.iter().map(|value| value.as_ref())),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )?;

        let mut scored_claims = Vec::new();
        for row in rows {
            let (
                id,
                subject,
                predicate,
                object,
                provenance,
                confidence,
                status,
                source_refs,
                agent_id,
                agent_kind,
                write_ts,
                last_verified_at,
                scope,
            ) = row?;
            let claim = Claim {
                id,
                subject,
                predicate,
                object,
                provenance: provenance.parse()?,
                confidence,
                status: status.parse()?,
                source_refs: serde_json::from_str(&source_refs)?,
                agent_id,
                agent_kind: agent_kind.parse()?,
                write_ts,
                last_verified_at,
                scope,
            };
            let score = recall_token_score(&query_tokens, &claim);
            if score > 0 {
                scored_claims.push((score, claim));
            }
        }
        let max_score = scored_claims
            .iter()
            .map(|(score, _)| *score)
            .max()
            .unwrap_or(0);
        let name_anchor_subject = if query_tokens.len() == 2 && max_score >= 4 {
            scored_claims
                .iter()
                .find(|(score, claim)| {
                    *score == max_score && claim.predicate.eq_ignore_ascii_case("name")
                })
                .map(|(_, claim)| claim.subject.clone())
        } else {
            None
        };
        let has_name_anchor = name_anchor_subject.is_some();
        let minimum_score = if has_name_anchor {
            2
        } else if max_score >= 5 {
            max_score - 1
        } else if max_score >= 3 {
            3
        } else if max_score >= 2 {
            2
        } else {
            1
        };
        scored_claims.retain(|(score, claim)| {
            *score >= minimum_score
                && name_anchor_subject
                    .as_ref()
                    .is_none_or(|subject| *score == max_score || claim.subject == *subject)
        });
        scored_claims.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        Ok(scored_claims.into_iter().map(|(_, claim)| claim).collect())
    }

    /// ADR-0023: retire a claim by flipping its status to INVALIDATED.
    /// Refuses if the claim does not exist or is already retired. The
    /// reason is appended to `source_refs` as `"retired:<reason>"` so the
    /// JSONL log + sqlite both retain provenance. The transition itself is
    /// logged as a `retire_claim` record before the projection UPDATE so
    /// replay does not resurrect retired claims as ACTIVE (ADR-0005).
    pub fn retire_claim(&self, claim_id: i64, reason: &str) -> Result<(), Error> {
        if reason.trim().is_empty() {
            return Err(Error::InvalidContradictionReason);
        }
        // The reason is written to the append-only log, so it must pass the
        // same privacy gate as every other persisted string.
        self.privacy_filter_recording(reason)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<(), Error> {
            let (status, source_refs_json): (String, String) = self
                .conn
                .query_row(
                    "SELECT status, source_refs FROM claims WHERE id = ?1",
                    [claim_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Error::MissingClaim { claim_id },
                    other => Error::Sqlite(other),
                })?;
            if status == "INVALIDATED" {
                return Err(Error::AlreadyRetired { claim_id });
            }
            let entry = RetireClaimLogEntry {
                kind: "retire_claim",
                ts: now,
                claim_id,
                reason,
            };
            append_jsonl(&self.log_path, &entry)?;
            let mut refs: Vec<String> = serde_json::from_str(&source_refs_json)?;
            refs.push(format!("retired:{reason}"));
            let new_refs = serde_json::to_string(&refs)?;
            self.conn.execute(
                "UPDATE claims
                    SET status = 'INVALIDATED',
                        source_refs = ?1,
                        last_seen_at = ?2
                  WHERE id = ?3",
                params![new_refs, now, claim_id],
            )?;
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
}
