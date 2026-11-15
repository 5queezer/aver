//! ADR-0032: claim validity (context and window), explicit reverify and the
//! evidence-since-confirmation count, derived on read.

use rusqlite::{Connection, OptionalExtension, named_params, params};

use crate::apply::apply;
use crate::error::Error;
use crate::log::{LogRecord, append_jsonl};
use crate::store::Store;
use crate::types::{
    AgentKind, Claim, ClaimConfirmation, ClaimStatus, ClaimWrite, ReverifyWrite, Verification,
};
use crate::validation::{validate_agent_id, validate_scope};

fn invalid_validity(detail: impl Into<String>) -> Error {
    Error::InvalidClaimValidity {
        detail: detail.into(),
    }
}

fn invalid_reverify(detail: impl Into<String>) -> Error {
    Error::InvalidReverify {
        detail: detail.into(),
    }
}

/// A window needs a context, starts no later than it ends, and the context
/// is a non-blank entity id.
pub(crate) fn validate_validity(write: &ClaimWrite) -> Result<(), Error> {
    match &write.context {
        Some(context) if context.trim().is_empty() => {
            return Err(invalid_validity("context must not be blank"));
        }
        None if write.valid_from.is_some() || write.valid_until.is_some() => {
            return Err(invalid_validity(
                "valid_from/valid_until need a context; set context to the entity the claim was established under",
            ));
        }
        _ => {}
    }
    if let (Some(from), Some(until)) = (write.valid_from, write.valid_until)
        && from > until
    {
        return Err(invalid_validity(format!(
            "valid_from {from} is after valid_until {until}"
        )));
    }
    Ok(())
}

/// `scope` equals `base` or lies below it (ADR-0021 descendants).
fn in_scope_set(base: &str, scope: &str) -> bool {
    scope
        .strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// `event:<id>` / `claim:<id>` evidence reference, if the string is one.
fn typed_reference(reference: &str) -> Option<(&'static str, &str)> {
    ["event:", "claim:"].into_iter().find_map(|prefix| {
        reference
            .strip_prefix(prefix)
            .map(|id| (&prefix[..prefix.len() - 1], id))
    })
}

impl Store {
    /// ADR-0032: record that `write.claim_id` was re-confirmed by the cited
    /// evidence. Checked before the log append: the claim is ACTIVE, the
    /// request scope is the claim's scope, there is evidence, every
    /// `event:<id>`/`claim:<id>` exists in the claim's scope or below it (and
    /// is not the claim itself), and `valid_until` only extends a window the
    /// claim has. Returns the logged verification time.
    pub fn reverify_claim(&self, write: ReverifyWrite) -> Result<i64, Error> {
        validate_scope(&write.scope)?;
        validate_agent_id(&write.agent_id)?;
        if write.evidence.is_empty() {
            return Err(invalid_reverify(
                "cite at least one evidence reference (event:<id>, claim:<id> or an item id)",
            ));
        }
        if write
            .evidence
            .iter()
            .any(|reference| reference.trim().is_empty())
        {
            return Err(invalid_reverify("evidence references must not be blank"));
        }
        self.privacy_filter_recording(&format!(
            "{} {} {}",
            write.agent_id,
            write.agent_kind.as_str(),
            write.evidence.join(" ")
        ))?;
        for reference in &write.evidence {
            self.privacy_filter_path_recording(reference)?;
        }

        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<(), Error> {
            // Under the write lock, so a concurrent retire or scope change
            // cannot slip between these checks and the append.
            let claim = self.reverify_target(&write)?;
            let record = LogRecord::ReverifyClaim {
                ts: now,
                claim_id: claim.id,
                verified_at: now,
                agent_id: Some(write.agent_id),
                agent_kind: Some(write.agent_kind.as_str().to_string()),
                evidence: write.evidence,
                valid_until: write.valid_until,
            };
            append_jsonl(&self.log_path, &record)?;
            apply(&self.conn, &record)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(now)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// The claim a confirmation targets, after the checks that read store
    /// state: ACTIVE, in the request scope, evidence present in its scope
    /// set, and a window end that only extends. Caller holds the write lock.
    fn reverify_target(&self, write: &ReverifyWrite) -> Result<Claim, Error> {
        let claim = self.get_claim(write.claim_id)?;
        if claim.status != ClaimStatus::Active {
            return Err(invalid_reverify(format!(
                "claim {} is {}; only ACTIVE claims can be confirmed",
                claim.id,
                claim.status.as_str()
            )));
        }
        if claim.scope != write.scope {
            return Err(invalid_reverify(format!(
                "request scope {:?} is not the claim's scope {:?}; confirm a claim from its own scope",
                write.scope, claim.scope
            )));
        }
        for reference in &write.evidence {
            let Some((kind, id)) = typed_reference(reference) else {
                continue;
            };
            let Some(id) = id.parse::<i64>().ok().filter(|id| *id > 0) else {
                return Err(invalid_reverify(format!(
                    "evidence {reference:?} is not {kind}:<positive id>"
                )));
            };
            let scope = match kind {
                "event" => self
                    .conn
                    .query_row(
                        "SELECT scope FROM episodic_events WHERE id = ?1",
                        [id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?,
                _ if id == claim.id => {
                    return Err(invalid_reverify(format!(
                        "claim {id} cannot cite itself as evidence"
                    )));
                }
                _ => self
                    .conn
                    .query_row("SELECT scope FROM claims WHERE id = ?1", [id], |row| {
                        row.get::<_, String>(0)
                    })
                    .optional()?,
            };
            if !scope.is_some_and(|scope| in_scope_set(&claim.scope, &scope)) {
                return Err(invalid_reverify(format!(
                    "evidence {reference:?} does not exist in scope {:?} or below it",
                    claim.scope
                )));
            }
        }
        if let Some(until) = write.valid_until {
            if claim.context.is_none() {
                return Err(invalid_reverify("valid_until needs a claim with a context"));
            }
            if let Some(from) = claim.valid_from
                && until < from
            {
                return Err(invalid_reverify(format!(
                    "valid_until {until} is before the window start {from}"
                )));
            }
            if let Some(current) = claim.valid_until
                && until < current
            {
                return Err(invalid_reverify(format!(
                    "valid_until only extends the window; it ends at {current}"
                )));
            }
        }
        Ok(claim)
    }

    /// ADR-0032: every explicit confirmation of a claim, oldest first.
    pub fn claim_verifications(&self, claim_id: i64) -> Result<Vec<Verification>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT verified_at, agent_id, agent_kind, evidence
               FROM claim_verifications WHERE claim_id = ?1 ORDER BY verified_at, id",
        )?;
        let rows = stmt
            .query_map([claim_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(verified_at, agent_id, agent_kind, evidence)| {
                Ok(Verification {
                    verified_at,
                    agent_id,
                    agent_kind: agent_kind.parse()?,
                    evidence: serde_json::from_str(&evidence)?,
                })
            })
            .collect()
    }

    /// ADR-0032: the claim's latest explicit confirmation and, for a claim
    /// with a context, the closed matching evidence in
    /// `(max(last_confirmed_at, valid_from), now]`. Reads only.
    pub fn claim_confirmation(&self, claim: &Claim, now: i64) -> Result<ClaimConfirmation, Error> {
        let (created_at, latest) = self.conn.query_row(
            "SELECT c.created_at, v.verified_at, v.agent_id, v.agent_kind, v.evidence
               FROM claims c
               LEFT JOIN claim_verifications v ON v.id = (
                    SELECT id FROM claim_verifications
                     WHERE claim_id = c.id
                     ORDER BY verified_at DESC, id DESC LIMIT 1)
              WHERE c.id = ?1",
            [claim.id],
            |row| {
                let created_at: i64 = row.get(0)?;
                let latest = match row.get::<_, Option<i64>>(1)? {
                    Some(verified_at) => Some((
                        verified_at,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    )),
                    None => None,
                };
                Ok((created_at, latest))
            },
        )?;
        let latest = latest
            .map(|(verified_at, agent_id, agent_kind, evidence)| {
                Ok::<_, Error>(Verification {
                    verified_at,
                    agent_id,
                    agent_kind: agent_kind.parse::<AgentKind>()?,
                    evidence: serde_json::from_str(&evidence)?,
                })
            })
            .transpose()?;
        let last_confirmed_at = latest
            .as_ref()
            .map_or(created_at, |v| v.verified_at.max(created_at));
        let evidence_since_confirmation = match &claim.context {
            Some(context) => {
                let baseline = claim
                    .valid_from
                    .map_or(last_confirmed_at, |from| from.max(last_confirmed_at));
                Some(self.count_closed_evidence(claim, context, baseline, now)?)
            }
            None => None,
        };
        Ok(ClaimConfirmation {
            last_confirmed_at,
            latest,
            evidence_since_confirmation,
        })
    }

    /// Distinct items E with ACTIVE claims, all in the claim's scope or below
    /// it: `E in_context <context>`, `E concerns <topic>` where the topics are
    /// the claim's subject plus the objects of its subject's own `concerns`
    /// claims, and `E closed_at …` created in `(baseline, now]`. Predicates
    /// include their descendants and aliases.
    fn count_closed_evidence(
        &self,
        claim: &Claim,
        context: &str,
        baseline: i64,
        now: i64,
    ) -> Result<i64, Error> {
        let names = |predicate: &str| -> Result<String, Error> {
            let mut names: Vec<String> = self
                .expand_predicate_filter(&[predicate])?
                .into_iter()
                .collect();
            names.sort();
            Ok(serde_json::to_string(&names)?)
        };
        let (concerns, in_context, closed_at) = (
            names("concerns")?,
            names("in_context")?,
            names("closed_at")?,
        );
        Ok(self.conn.query_row(
            "WITH topics(topic) AS (
                 SELECT :subject
                 UNION
                 SELECT object FROM claims
                  WHERE subject = :subject AND status = 'ACTIVE'
                    AND predicate IN (SELECT value FROM json_each(:concerns))
                    AND (scope = :scope OR substr(scope, 1, length(:scope) + 1) = :scope || '/')
             )
             SELECT COUNT(DISTINCT item.subject)
               FROM claims item
              WHERE item.status = 'ACTIVE' AND item.object = :context
                AND item.predicate IN (SELECT value FROM json_each(:in_context))
                AND (item.scope = :scope OR substr(item.scope, 1, length(:scope) + 1) = :scope || '/')
                AND EXISTS (
                    SELECT 1 FROM claims tag
                     WHERE tag.subject = item.subject AND tag.status = 'ACTIVE'
                       AND tag.predicate IN (SELECT value FROM json_each(:concerns))
                       AND tag.object IN (SELECT topic FROM topics)
                       AND (tag.scope = :scope OR substr(tag.scope, 1, length(:scope) + 1) = :scope || '/'))
                AND EXISTS (
                    SELECT 1 FROM claims done
                     WHERE done.subject = item.subject AND done.status = 'ACTIVE'
                       AND done.predicate IN (SELECT value FROM json_each(:closed_at))
                       AND done.created_at > :baseline AND done.created_at <= :now
                       AND (done.scope = :scope OR substr(done.scope, 1, length(:scope) + 1) = :scope || '/'))",
            named_params! {
                ":subject": claim.subject,
                ":scope": claim.scope,
                ":context": context,
                ":concerns": concerns,
                ":in_context": in_context,
                ":closed_at": closed_at,
                ":baseline": baseline,
                ":now": now,
            },
            |row| row.get(0),
        )?)
    }
}

/// Explicit verifications whose `event:`/`claim:` evidence is missing, the
/// claim itself, or outside the claim's scope set. Replay refuses a
/// projection with any (events load after claims, so this runs at the end).
pub(crate) fn invalid_evidence_links(conn: &Connection) -> Result<i64, Error> {
    Ok(conn.query_row(
        "SELECT COUNT(*)
           FROM claim_verifications v
           JOIN claims c ON c.id = v.claim_id, json_each(v.evidence) j
          WHERE (substr(j.value, 1, 6) = 'event:' AND NOT EXISTS (
                    SELECT 1 FROM episodic_events e
                     WHERE 'event:' || e.id = j.value
                       AND (e.scope = c.scope OR substr(e.scope, 1, length(c.scope) + 1) = c.scope || '/')))
             OR (substr(j.value, 1, 6) = 'claim:' AND NOT EXISTS (
                    SELECT 1 FROM claims x
                     WHERE 'claim:' || x.id = j.value AND x.id <> c.id
                       AND (x.scope = c.scope OR substr(x.scope, 1, length(c.scope) + 1) = c.scope || '/')))",
        params![],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::{in_scope_set, typed_reference};

    #[test]
    fn scope_set_is_the_scope_and_its_descendants() {
        assert!(in_scope_set("a", "a"));
        assert!(in_scope_set("a", "a/b"));
        assert!(!in_scope_set("a", "ab"));
        assert!(!in_scope_set("a/b", "a"));
        assert!(!in_scope_set("a", "replay/r"));
    }

    #[test]
    fn typed_references_are_event_and_claim_prefixes() {
        assert_eq!(typed_reference("event:3"), Some(("event", "3")));
        assert_eq!(typed_reference("claim:x"), Some(("claim", "x")));
        assert_eq!(typed_reference("plot:7"), None);
    }
}
