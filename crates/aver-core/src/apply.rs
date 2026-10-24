//! The one projection writer (ADR-0025): [`apply`] is the only code in the
//! crate that mutates the SQLite tables holding logged state — claims,
//! hyperedges (+participants), episodic_events, observations,
//! observation_prune_markers, candidate_claims, contradictions,
//! vector_chunks (+vector_index) — and their derived side effects (entity
//! rows, ontology extension, candidate status flips, lifecycle updates).
//!
//! Live writes call it right after `append_jsonl`; replay calls it per log
//! line. It is deterministic: every id and timestamp comes from the record,
//! nothing is read from the clock, and the privacy filter does not run here
//! (ADR-0019 §4: the log is already filtered).
//!
//! Idempotency: a record whose id already exists with identical content is
//! reported as [`Applied::Duplicate`] and changes nothing; the same id with
//! different content is [`Error::ReplayDuplicateId`]. References to rows
//! that do not exist are [`Error::InvalidRecord`].

use rusqlite::{Connection, OptionalExtension, params};

use crate::claims::provenance_for_agent_kind;
use crate::error::Error;
use crate::log::{Decision, LogRecord};
use crate::ontology::ensure_entity_on;
use crate::seed;
use crate::store::VECTOR_INDEX_DIM;
use crate::types::{AgentKind, HyperedgeParticipantInput, Provenance};

/// Outcome of applying one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applied {
    /// Projection rows were written.
    Applied,
    /// An identical record was already applied; nothing changed.
    Duplicate,
}

fn invalid(detail: impl Into<String>) -> Error {
    Error::InvalidRecord {
        detail: detail.into(),
    }
}

fn duplicate(detail: impl Into<String>) -> Error {
    Error::ReplayDuplicateId {
        detail: detail.into(),
    }
}

fn has_table(conn: &Connection, name: &str) -> Result<bool, Error> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Apply one log record to the projection. See the module docs.
pub(crate) fn apply(conn: &Connection, record: &LogRecord) -> Result<Applied, Error> {
    match record {
        LogRecord::AddClaim {
            ts,
            claim_id,
            subject,
            predicate,
            object,
            source,
            agent_id,
            agent_kind,
            confidence,
            provenance,
            scope,
        } => {
            let agent_kind: AgentKind = agent_kind.parse()?;
            let provenance = match provenance {
                Some(raw) => raw
                    .parse::<Provenance>()
                    .map_err(|_| invalid(format!("unknown 'provenance' value: {raw}")))?,
                None => provenance_for_agent_kind(agent_kind),
            };
            add_claim(
                conn,
                *ts,
                *claim_id,
                subject,
                predicate,
                object,
                source,
                agent_id,
                agent_kind,
                *confidence,
                provenance,
                scope,
            )
        }
        LogRecord::AddHyperedge {
            ts,
            hyperedge_id,
            predicate,
            provenance,
            confidence,
            source_refs,
            participants,
        } => add_hyperedge(
            conn,
            *ts,
            *hyperedge_id,
            predicate,
            provenance.parse()?,
            *confidence,
            source_refs,
            participants,
        ),
        LogRecord::RecordEvent {
            ts,
            event_id,
            session_id,
            event_kind,
            payload,
            source,
            agent_id,
            agent_kind,
            scope,
        } => record_event(
            conn, *ts, *event_id, session_id, event_kind, payload, source, agent_id, agent_kind,
            scope,
        ),
        LogRecord::RecordObservation {
            ts,
            observation_id,
            session_id,
            content,
            relevance,
            source_event_ids,
            agent_id,
            agent_kind,
            derivation,
            scope,
        } => record_observation(
            conn,
            *ts,
            observation_id,
            session_id,
            content,
            relevance,
            source_event_ids,
            agent_id,
            agent_kind,
            derivation,
            scope,
        ),
        LogRecord::PruneObservations {
            ts,
            prune_marker_id,
            session_id,
            pruned_observation_ids,
        } => prune_observations(
            conn,
            *ts,
            prune_marker_id,
            session_id,
            pruned_observation_ids,
        ),
        LogRecord::RetireClaim {
            ts,
            claim_id,
            reason,
        } => retire_claim(conn, *ts, *claim_id, reason),
        LogRecord::AddContradiction {
            ts,
            contradiction_id,
            claim_id,
            reason,
            new_claim_id,
        } => add_contradiction(
            conn,
            *ts,
            *contradiction_id,
            *claim_id,
            reason,
            *new_claim_id,
        ),
        LogRecord::ProposeCandidateClaim {
            ts,
            candidate_id,
            event_id,
            subject,
            predicate,
            object,
            scope,
        } => propose_candidate_claim(
            conn,
            *ts,
            *candidate_id,
            *event_id,
            subject,
            predicate,
            object,
            scope,
        ),
        LogRecord::PromoteCandidateClaim {
            candidate_id,
            claim_id,
            decision,
            ..
        } => {
            check_decision(decision.as_ref(), "promote")?;
            promote_candidate_claim(conn, *candidate_id, *claim_id)
        }
        LogRecord::RejectCandidateClaim {
            candidate_id,
            reason,
            decision,
            ..
        } => {
            check_decision(decision.as_ref(), "reject")?;
            reject_candidate_claim(conn, *candidate_id, reason)
        }
        LogRecord::SupersedeClaims { claim_ids, .. } => supersede_claims(conn, claim_ids),
        LogRecord::DecayConfidence { changes, .. } => {
            for change in changes {
                let rows_changed = conn.execute(
                    "UPDATE claims SET confidence = ?1 WHERE id = ?2",
                    params![change.confidence, change.claim_id],
                )?;
                if rows_changed != 1 {
                    return Err(invalid(format!(
                        "decay_confidence references missing claim {}",
                        change.claim_id
                    )));
                }
            }
            Ok(Applied::Applied)
        }
        LogRecord::MergeSourceRefs {
            claim_id,
            source_refs,
            promote,
            ..
        } => merge_source_refs(conn, *claim_id, source_refs, *promote),
        LogRecord::AddVectorChunk {
            ts,
            chunk_id,
            claim_id,
            text,
            embedding_model,
            embedding,
        } => add_vector_chunk(
            conn,
            *ts,
            *chunk_id,
            *claim_id,
            text,
            embedding_model,
            embedding.as_deref(),
        ),
        LogRecord::EmbedVectorChunk {
            chunk_id,
            embedding,
            ..
        } => embed_vector_chunk(conn, *chunk_id, embedding),
    }
}

/// ADR-0018: make `predicate` known to the ontology if it is neither a
/// predicate type nor an alias. The live writer only reaches this point for
/// USER_ASSERTED claims (`Store::validate_ontology` rejects every other
/// unknown predicate before the record is logged), so `created_via` is
/// `user_assertion` for those; records from older logs that were accepted
/// under a different ontology are marked `replay`. Either way the log is the
/// source of truth (ADR-0005) and the row is extended, never rejected.
fn ensure_predicate_known(
    conn: &Connection,
    predicate: &str,
    provenance: Provenance,
    agent_id: &str,
    ts: i64,
) -> Result<(), Error> {
    let known: Option<i64> = conn
        .query_row(
            "SELECT id FROM predicate_types WHERE name = ?1",
            [predicate],
            |row| row.get(0),
        )
        .optional()?;
    if known.is_some() {
        return Ok(());
    }
    let alias_hit: Option<i64> = conn
        .query_row(
            "SELECT predicate_id FROM predicate_alias WHERE alias = ?1",
            [predicate],
            |row| row.get(0),
        )
        .optional()?;
    if alias_hit.is_some() {
        return Ok(());
    }
    let parent_id: i64 = conn.query_row(
        "SELECT id FROM predicate_types WHERE name = 'relates_to'",
        [],
        |row| row.get(0),
    )?;
    let created_via = if provenance == Provenance::UserAsserted {
        "user_assertion"
    } else {
        "replay"
    };
    conn.execute(
        "INSERT INTO predicate_types (name, parent_id, created_via, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![predicate, parent_id, created_via, ts],
    )?;
    seed::rebuild_closure(conn, "predicate_types", "predicate_closure")?;
    conn.execute(
        "INSERT INTO ontology_extension_log (predicate, parent, agent_id, created_at)
         VALUES (?1, 'relates_to', ?2, ?3)",
        params![predicate, agent_id, ts],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_claim(
    conn: &Connection,
    ts: i64,
    claim_id: i64,
    subject: &str,
    predicate: &str,
    object: &str,
    source: &str,
    agent_id: &str,
    agent_kind: AgentKind,
    confidence: f64,
    provenance: Provenance,
    scope: &str,
) -> Result<Applied, Error> {
    // Only write-time immutable fields are compared: lifecycle records
    // (decay, merge, retire) legitimately change provenance/confidence/
    // source_refs/status after the add_claim line.
    let existing: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT subject, predicate, object, agent_id, agent_kind, scope
               FROM claims WHERE id = ?1",
            [claim_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == subject
            && existing.1 == predicate
            && existing.2 == object
            && existing.3 == agent_id
            && existing.4 == agent_kind.as_str()
            && existing.5 == scope;
        if !matches {
            return Err(duplicate(format!("claim_id={claim_id} content mismatch")));
        }
        return Ok(Applied::Duplicate);
    }

    ensure_predicate_known(conn, predicate, provenance, agent_id, ts)?;
    ensure_entity_on(conn, subject, ts)?;
    ensure_entity_on(conn, object, ts)?;

    let source_refs = serde_json::to_string(&[source])?;
    conn.execute(
        "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                             status, source_refs, agent_id, agent_kind, write_ts,
                             created_at, last_seen_at, last_verified_at, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7,
                 ?8, ?9, ?10, ?10, ?10, ?10, ?11)",
        params![
            claim_id,
            subject,
            predicate,
            object,
            provenance.as_str(),
            confidence,
            source_refs,
            agent_id,
            agent_kind.as_str(),
            ts,
            scope
        ],
    )?;
    Ok(Applied::Applied)
}

#[allow(clippy::too_many_arguments)]
fn add_hyperedge(
    conn: &Connection,
    ts: i64,
    hyperedge_id: i64,
    predicate: &str,
    provenance: Provenance,
    confidence: f64,
    source_refs: &[String],
    participants: &[HyperedgeParticipantInput],
) -> Result<Applied, Error> {
    let existing: Option<(String, String, f64, String)> = conn
        .query_row(
            "SELECT predicate, provenance, confidence, source_refs
               FROM hyperedges WHERE id = ?1",
            [hyperedge_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let existing_sources: Vec<String> = serde_json::from_str(&existing.3)?;
        let existing_participants = hyperedge_participants(conn, hyperedge_id)?;
        let matches = existing.0 == predicate
            && existing.1 == provenance.as_str()
            && (existing.2 - confidence).abs() < 1e-9
            && existing_sources == source_refs
            && existing_participants == participants;
        if !matches {
            return Err(duplicate(format!(
                "hyperedge_id={hyperedge_id} content mismatch"
            )));
        }
        return Ok(Applied::Duplicate);
    }

    ensure_predicate_known(conn, predicate, provenance, "local", ts)?;
    let source_refs_json = serde_json::to_string(source_refs)?;
    conn.execute(
        "INSERT INTO hyperedges (id, predicate, provenance, confidence, source_refs, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'ACTIVE', ?6, ?6)",
        params![
            hyperedge_id,
            predicate,
            provenance.as_str(),
            confidence,
            source_refs_json,
            ts,
        ],
    )?;
    for participant in participants {
        ensure_entity_on(conn, &participant.entity, ts)?;
        conn.execute(
            "INSERT INTO hyperedge_participants (hyperedge_id, role, entity)
             VALUES (?1, ?2, ?3)",
            params![hyperedge_id, participant.role, participant.entity],
        )?;
    }
    Ok(Applied::Applied)
}

fn hyperedge_participants(
    conn: &Connection,
    hyperedge_id: i64,
) -> Result<Vec<HyperedgeParticipantInput>, Error> {
    let mut stmt = conn.prepare(
        "SELECT role, entity
           FROM hyperedge_participants
          WHERE hyperedge_id = ?1
          ORDER BY id",
    )?;
    Ok(stmt
        .query_map([hyperedge_id], |row| {
            Ok(HyperedgeParticipantInput {
                role: row.get(0)?,
                entity: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    conn: &Connection,
    ts: i64,
    event_id: i64,
    session_id: &str,
    event_kind: &str,
    payload: &str,
    source: &str,
    agent_id: &str,
    agent_kind: &str,
    scope: &str,
) -> Result<Applied, Error> {
    let existing: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT session_id, kind, payload, source, agent_id, agent_kind
               FROM episodic_events WHERE id = ?1",
            [event_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id
            && existing.1 == event_kind
            && existing.2 == payload
            && existing.3 == source
            && existing.4 == agent_id
            && existing.5 == agent_kind;
        if !matches {
            return Err(duplicate(format!("event_id={event_id} content mismatch")));
        }
        return Ok(Applied::Duplicate);
    }
    conn.execute(
        "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event_id, session_id, event_kind, payload, source, agent_id, agent_kind, ts, scope
        ],
    )?;
    Ok(Applied::Applied)
}

#[allow(clippy::too_many_arguments)]
fn record_observation(
    conn: &Connection,
    ts: i64,
    id: &str,
    session_id: &str,
    content: &str,
    relevance: &str,
    source_event_ids: &[i64],
    agent_id: &str,
    agent_kind: &str,
    derivation: &str,
    scope: &str,
) -> Result<Applied, Error> {
    let source_event_ids_json = serde_json::to_string(source_event_ids)?;
    let existing: Option<(String, String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation
               FROM observations WHERE id = ?1",
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
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id
            && existing.1 == content
            && existing.2 == relevance
            && existing.3 == source_event_ids_json
            && existing.4 == agent_id
            && existing.5 == agent_kind
            && existing.6 == derivation;
        if !matches {
            return Err(duplicate(format!("observation_id={id} content mismatch")));
        }
        return Ok(Applied::Duplicate);
    }
    conn.execute(
        "INSERT INTO observations
         (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            session_id,
            content,
            relevance,
            source_event_ids_json,
            agent_id,
            agent_kind,
            derivation,
            ts,
            scope
        ],
    )?;
    Ok(Applied::Applied)
}

fn prune_observations(
    conn: &Connection,
    ts: i64,
    marker_id: &str,
    session_id: &str,
    pruned_observation_ids: &[String],
) -> Result<Applied, Error> {
    let pruned_json = serde_json::to_string(pruned_observation_ids)?;
    let existing: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT session_id, pruned_observation_ids, ts
               FROM observation_prune_markers WHERE id = ?1",
            [marker_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == session_id && existing.1 == pruned_json && existing.2 == ts;
        if !matches {
            return Err(duplicate(format!(
                "prune_marker_id={marker_id} content mismatch"
            )));
        }
        return Ok(Applied::Duplicate);
    }
    conn.execute(
        "INSERT INTO observation_prune_markers
         (id, session_id, pruned_observation_ids, ts)
         VALUES (?1, ?2, ?3, ?4)",
        params![marker_id, session_id, pruned_json, ts],
    )?;
    Ok(Applied::Applied)
}

/// ADR-0023: append the `retired:<reason>` source_refs marker and flip the
/// claim to INVALIDATED. A second identical retirement is a duplicate; a
/// different reason on an already-retired claim is a conflict.
fn retire_claim(conn: &Connection, ts: i64, claim_id: i64, reason: &str) -> Result<Applied, Error> {
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT status, source_refs FROM claims WHERE id = ?1",
            [claim_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, source_refs_json)) = existing else {
        return Err(invalid(format!(
            "retire_claim references missing claim {claim_id}"
        )));
    };
    let marker = format!("retired:{reason}");
    let mut refs: Vec<String> = serde_json::from_str(&source_refs_json)?;
    if status == "INVALIDATED" {
        if refs.iter().any(|source_ref| source_ref == &marker) {
            return Ok(Applied::Duplicate);
        }
        return Err(duplicate(format!(
            "claim_id={claim_id} already INVALIDATED"
        )));
    }
    refs.push(marker);
    conn.execute(
        "UPDATE claims
            SET status = 'INVALIDATED',
                source_refs = ?1,
                last_seen_at = ?2
          WHERE id = ?3",
        params![serde_json::to_string(&refs)?, ts, claim_id],
    )?;
    Ok(Applied::Applied)
}

fn add_contradiction(
    conn: &Connection,
    ts: i64,
    contradiction_id: i64,
    claim_id: i64,
    reason: &str,
    new_claim_id: Option<i64>,
) -> Result<Applied, Error> {
    let existing: Option<(i64, String, Option<i64>)> = conn
        .query_row(
            "SELECT claim_id, reason, new_claim_id FROM contradictions WHERE id = ?1",
            [contradiction_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == claim_id && existing.1 == reason && existing.2 == new_claim_id;
        if !matches {
            return Err(duplicate(format!(
                "contradiction_id={contradiction_id} content mismatch"
            )));
        }
        return Ok(Applied::Duplicate);
    }
    conn.execute(
        "INSERT INTO contradictions (id, claim_id, reason, new_claim_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![contradiction_id, claim_id, reason, new_claim_id, ts],
    )?;
    Ok(Applied::Applied)
}

#[allow(clippy::too_many_arguments)]
fn propose_candidate_claim(
    conn: &Connection,
    ts: i64,
    candidate_id: i64,
    event_id: i64,
    subject: &str,
    predicate: &str,
    object: &str,
    scope: &str,
) -> Result<Applied, Error> {
    let existing: Option<(i64, String, String, String, String)> = conn
        .query_row(
            "SELECT event_id, subject, predicate, object, scope
               FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == event_id
            && existing.1 == subject
            && existing.2 == predicate
            && existing.3 == object
            && existing.4 == scope;
        if !matches {
            return Err(duplicate(format!(
                "candidate_id={candidate_id} content mismatch"
            )));
        }
        return Ok(Applied::Duplicate);
    }
    // provenance/confidence/status stay at the schema defaults (INFERRED,
    // 0.45, PENDING) — the proposal path never overrides them.
    conn.execute(
        "INSERT INTO candidate_claims (id, event_id, subject, predicate, object, created_at, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            candidate_id,
            event_id,
            subject,
            predicate,
            object,
            ts,
            scope
        ],
    )?;
    Ok(Applied::Applied)
}

/// ADR-0024: an absent `decision` is fine (manual transitions and older
/// lines); a present one must name a model and the choice matching the kind.
fn check_decision(decision: Option<&Decision>, expected_choice: &str) -> Result<(), Error> {
    match decision {
        None => Ok(()),
        Some(decision)
            if !decision.model.trim().is_empty() && decision.choice == expected_choice =>
        {
            Ok(())
        }
        Some(decision) => Err(invalid(format!(
            "'decision' must be {{\"model\": <non-empty string>, \"choice\": \"{expected_choice}\"}}, got {}",
            serde_json::to_string(decision)?
        ))),
    }
}

fn promote_candidate_claim(
    conn: &Connection,
    candidate_id: i64,
    claim_id: i64,
) -> Result<Applied, Error> {
    let existing: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT status, promoted_claim_id FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, promoted_claim_id)) = existing else {
        return Err(invalid(format!(
            "promote_candidate_claim references missing candidate {candidate_id}"
        )));
    };
    if status == "PROMOTED" {
        if promoted_claim_id == Some(claim_id) {
            return Ok(Applied::Duplicate);
        }
        return Err(duplicate(format!(
            "candidate_id={candidate_id} already PROMOTED to a different claim"
        )));
    }
    if status != "PENDING" {
        return Err(invalid(format!(
            "cannot promote candidate {candidate_id} from status {status}"
        )));
    }
    let claim_exists = conn
        .query_row("SELECT 1 FROM claims WHERE id = ?1", [claim_id], |_| Ok(()))
        .optional()?
        .is_some();
    if !claim_exists {
        return Err(invalid(format!(
            "promotion references missing claim {claim_id}"
        )));
    }
    let rows_changed = conn.execute(
        "UPDATE candidate_claims
            SET status = 'PROMOTED', promoted_claim_id = ?1
          WHERE id = ?2 AND status = 'PENDING'",
        params![claim_id, candidate_id],
    )?;
    if rows_changed != 1 {
        return Err(invalid(format!(
            "promotion changed {rows_changed} rows for candidate {candidate_id}"
        )));
    }
    Ok(Applied::Applied)
}

fn reject_candidate_claim(
    conn: &Connection,
    candidate_id: i64,
    reason: &str,
) -> Result<Applied, Error> {
    let existing: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, rejection_reason FROM candidate_claims WHERE id = ?1",
            [candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((status, rejection_reason)) = existing else {
        return Err(invalid(format!(
            "reject_candidate_claim references missing candidate {candidate_id}"
        )));
    };
    if status == "REJECTED" {
        if rejection_reason.as_deref() == Some(reason) {
            return Ok(Applied::Duplicate);
        }
        return Err(duplicate(format!(
            "candidate_id={candidate_id} already REJECTED with a different reason"
        )));
    }
    if status != "PENDING" {
        return Err(invalid(format!(
            "cannot reject candidate {candidate_id} from status {status}"
        )));
    }
    let rows_changed = conn.execute(
        "UPDATE candidate_claims
            SET status = 'REJECTED', rejection_reason = ?1
          WHERE id = ?2 AND status = 'PENDING'",
        params![reason, candidate_id],
    )?;
    if rows_changed != 1 {
        return Err(invalid(format!(
            "rejection changed {rows_changed} rows for candidate {candidate_id}"
        )));
    }
    Ok(Applied::Applied)
}

/// Naturally idempotent: a claim already SUPERSEDED is skipped.
fn supersede_claims(conn: &Connection, claim_ids: &[i64]) -> Result<Applied, Error> {
    for claim_id in claim_ids {
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM claims WHERE id = ?1",
                [claim_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Err(invalid(format!(
                "supersede_claims references missing claim {claim_id}"
            )));
        };
        if status == "SUPERSEDED" {
            continue;
        }
        let rows_changed = conn.execute(
            "UPDATE claims SET status = 'SUPERSEDED' WHERE id = ?1",
            [claim_id],
        )?;
        if rows_changed != 1 {
            return Err(invalid(format!(
                "supersede changed {rows_changed} rows for claim {claim_id}"
            )));
        }
    }
    Ok(Applied::Applied)
}

fn merge_source_refs(
    conn: &Connection,
    claim_id: i64,
    source_refs: &[String],
    promote: bool,
) -> Result<Applied, Error> {
    let merged = serde_json::to_string(source_refs)?;
    let rows_changed = if promote {
        conn.execute(
            "UPDATE claims
                SET source_refs = ?1, provenance = 'EXTRACTED', confidence = MAX(confidence, 0.75)
              WHERE id = ?2",
            params![merged, claim_id],
        )?
    } else {
        conn.execute(
            "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
            params![merged, claim_id],
        )?
    };
    if rows_changed != 1 {
        return Err(invalid(format!(
            "merge_source_refs references missing claim {claim_id}"
        )));
    }
    Ok(Applied::Applied)
}

/// ADR-0017 §"Populate strategy": the `vec0` index only takes vectors of the
/// canonical dimension; off-dimension rows stay in `vector_chunks` and are
/// served by the JSON full-scan fallback.
fn index_embedding(
    conn: &Connection,
    chunk_id: i64,
    embedding: &[f32],
    embedding_json: &str,
) -> Result<(), Error> {
    if embedding.len() == VECTOR_INDEX_DIM && has_table(conn, "vector_index")? {
        conn.execute(
            "INSERT OR IGNORE INTO vector_index(chunk_id, embedding) VALUES (?1, ?2)",
            params![chunk_id, embedding_json],
        )?;
    }
    Ok(())
}

fn add_vector_chunk(
    conn: &Connection,
    ts: i64,
    chunk_id: i64,
    claim_id: i64,
    text: &str,
    embedding_model: &str,
    embedding: Option<&[f32]>,
) -> Result<Applied, Error> {
    let embedding_json = embedding.map(serde_json::to_string).transpose()?;
    let existing: Option<(i64, String, String, Option<String>)> = conn
        .query_row(
            "SELECT claim_id, text, embedding_model, embedding_json
               FROM vector_chunks WHERE id = ?1",
            [chunk_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some(existing) = existing {
        let matches = existing.0 == claim_id
            && existing.1 == text
            && existing.2 == embedding_model
            && existing.3 == embedding_json;
        if !matches {
            return Err(duplicate(format!("chunk_id={chunk_id} content mismatch")));
        }
        return Ok(Applied::Duplicate);
    }
    conn.execute(
        "INSERT INTO vector_chunks (id, claim_id, text, embedding_model, embedding_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![chunk_id, claim_id, text, embedding_model, embedding_json, ts],
    )?;
    if let (Some(embedding), Some(json)) = (embedding, &embedding_json) {
        index_embedding(conn, chunk_id, embedding, json)?;
    }
    Ok(Applied::Applied)
}

fn embed_vector_chunk(
    conn: &Connection,
    chunk_id: i64,
    embedding: &[f32],
) -> Result<Applied, Error> {
    let embedding_json = serde_json::to_string(embedding)?;
    let existing: Option<Option<String>> = conn
        .query_row(
            "SELECT embedding_json FROM vector_chunks WHERE id = ?1",
            [chunk_id],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        None => Err(invalid(format!(
            "embed_vector_chunk references missing chunk {chunk_id}"
        ))),
        Some(Some(current)) if current == embedding_json => Ok(Applied::Duplicate),
        Some(Some(_)) => Err(duplicate(format!(
            "chunk_id={chunk_id} already has a different embedding"
        ))),
        Some(None) => {
            conn.execute(
                "UPDATE vector_chunks SET embedding_json = ?1 WHERE id = ?2",
                params![embedding_json, chunk_id],
            )?;
            index_embedding(conn, chunk_id, embedding, &embedding_json)?;
            Ok(Applied::Applied)
        }
    }
}
