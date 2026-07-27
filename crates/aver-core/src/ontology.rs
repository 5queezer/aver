//! Predicate/entity-type vocabulary, closure, and entity typing.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;
use crate::seed;
use crate::store::Store;
use crate::types::Provenance;
use crate::validation::validate_claim_field;

pub(crate) const UNKNOWN_PREDICATE_LIST_LIMIT: usize = 32;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PredicateCandidate {
    accepted: String,
    canonical: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PredicateSuggestion {
    accepted: String,
    canonical: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PredicateVocabulary {
    predicates: Vec<String>,
    aliases: Vec<String>,
    candidates: Vec<PredicateCandidate>,
}

pub(crate) fn normalize_predicate_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            _ => ch.to_ascii_lowercase(),
        })
        .collect()
}

pub(crate) fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let deletion = previous[right_index + 1] + 1;
            let insertion = current[right_index] + 1;
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            current[right_index + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

pub(crate) fn semantic_predicate_hint(normalized: &str) -> Option<&'static str> {
    match normalized {
        "deprioritizes" | "prioritizes" | "prioritises" | "favors" | "favours" | "likes"
        | "preference" => Some("prefers"),
        "requires" | "needs" | "needed_by" | "depends" | "depends_upon" => Some("depends_on"),
        "about" | "related_to" | "associated_with" => Some("relates_to"),
        "owns" | "owner" | "owned" => Some("owns"),
        "tests" | "tested_by" | "validates" => Some("tests"),
        "fixes" | "resolves" | "repairs" => Some("fixes"),
        _ => None,
    }
}

pub(crate) fn suggest_unknown_predicate(
    name: &str,
    candidates: &[PredicateCandidate],
) -> Option<PredicateSuggestion> {
    let normalized = normalize_predicate_name(name);
    if let Some(semantic_hint) = semantic_predicate_hint(&normalized)
        && let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.accepted == semantic_hint)
    {
        return Some(PredicateSuggestion {
            accepted: candidate.accepted.clone(),
            canonical: candidate.canonical.clone(),
        });
    }

    let mut best: Option<(&PredicateCandidate, usize)> = None;

    for candidate in candidates {
        let candidate_normalized = normalize_predicate_name(&candidate.accepted);
        let distance = edit_distance(&normalized, &candidate_normalized);
        let is_better = match best {
            None => true,
            Some((best_candidate, best_distance)) => {
                distance < best_distance
                    || (distance == best_distance
                        && candidate.accepted.len() < best_candidate.accepted.len())
                    || (distance == best_distance
                        && candidate.accepted.len() == best_candidate.accepted.len()
                        && candidate.accepted < best_candidate.accepted)
            }
        };
        if is_better {
            best = Some((candidate, distance));
        }
    }

    best.and_then(|(candidate, distance)| {
        let threshold = match normalized.len().max(candidate.accepted.len()) {
            0..=4 => 1,
            5..=8 => 2,
            _ => 3,
        };
        (distance <= threshold).then(|| PredicateSuggestion {
            accepted: candidate.accepted.clone(),
            canonical: candidate.canonical.clone(),
        })
    })
}

pub(crate) fn format_available_values(values: &[String]) -> String {
    let mut formatted = values
        .iter()
        .take(UNKNOWN_PREDICATE_LIST_LIMIT)
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>();
    let remaining = values.len().saturating_sub(UNKNOWN_PREDICATE_LIST_LIMIT);
    if remaining > 0 {
        formatted.push(format!("… and {remaining} more"));
    }
    formatted.join(", ")
}

pub(crate) fn format_unknown_predicate(
    name: &str,
    suggestion: Option<&PredicateSuggestion>,
    available_predicates: &[String],
    available_aliases: &[String],
) -> String {
    let mut message =
        format!("unknown predicate: {name} (not in predicate_types or predicate_alias).");
    if let Some(suggestion) = suggestion {
        match suggestion.canonical.as_deref() {
            Some(canonical) if canonical != suggestion.accepted => message.push_str(&format!(
                " did you mean `{}` (alias for `{canonical}`)?",
                suggestion.accepted
            )),
            _ => message.push_str(&format!(" did you mean `{}`?", suggestion.accepted)),
        }
    }
    if !available_predicates.is_empty() {
        message.push_str(" available predicates: ");
        message.push_str(&format_available_values(available_predicates));
        message.push('.');
    }
    if !available_aliases.is_empty() {
        message.push_str(" accepted aliases: ");
        message.push_str(&format_available_values(available_aliases));
        message.push('.');
    }
    if let Some(suggestion) = suggestion {
        message.push_str(&format!(
            " retry hint: safe to retry with predicate `{}`; do not invent predicates or silently rewrite the stored claim.",
            suggestion.accepted
        ));
    } else {
        message.push_str(" retry hint: choose one available predicate or alias, then retry; `relates_to` is the safest generic fallback when no precise predicate fits.");
    }
    message
}

pub(crate) fn entity_type_id_on(conn: &Connection, name: &str) -> Result<Option<i64>, Error> {
    conn.query_row(
        "SELECT id FROM entity_types WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .optional()
    .map_err(Error::Sqlite)
}

pub(crate) fn infer_entity_type_name_on(conn: &Connection, entity: &str) -> Result<String, Error> {
    if let Some((prefix, _rest)) = entity.split_once(':')
        && entity_type_id_on(conn, prefix)?.is_some()
    {
        return Ok(prefix.to_string());
    }
    match entity {
        "User" => Ok("Human".to_string()),
        "Claude" | "Pi" => Ok("Bot".to_string()),
        _ => Ok("Thing".to_string()),
    }
}

/// Single `ensure_entity` implementation shared by `Store` writes and log
/// replay (ADR-0018 §"Subject/object policy"). Replay must reproduce the
/// same entity classifications (`prefix:` inference, Thing fallback,
/// requires_review flag) as the live write path; a replayed DB that typed
/// every entity as Thing would silently lose that information.
pub(crate) fn ensure_entity_on(conn: &Connection, entity: &str, now: i64) -> Result<(), Error> {
    let inferred_type = infer_entity_type_name_on(conn, entity)?;
    // `Thing` is seeded by the ontology bootstrap; a missing row means a
    // corrupt or seedless database — report it instead of panicking.
    let thing_id =
        entity_type_id_on(conn, "Thing")?.ok_or(Error::MissingEntityType { name: "Thing" })?;
    let type_id = entity_type_id_on(conn, &inferred_type)?.unwrap_or(thing_id);
    // When the inferred type falls back to `Thing` (no `prefix:` and no
    // synonym match), surface the entity for consolidation review instead of
    // silently coercing.
    let requires_review = if type_id == thing_id { 1_i64 } else { 0_i64 };
    let current: Option<i64> = conn
        .query_row(
            "SELECT type_id FROM entities WHERE name = ?1",
            [entity],
            |row| row.get(0),
        )
        .optional()?;
    match current {
        None => {
            conn.execute(
                "INSERT INTO entities (name, type_id, requires_review, created_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![entity, type_id, requires_review, now],
            )?;
        }
        Some(existing) if existing == thing_id && type_id != thing_id => {
            // Promotion from Thing → real type clears the review flag.
            conn.execute(
                "UPDATE entities
                    SET type_id = ?2, requires_review = 0, last_seen_at = ?3
                  WHERE name = ?1",
                params![entity, type_id, now],
            )?;
        }
        Some(_) => {
            conn.execute(
                "UPDATE entities SET last_seen_at = ?2 WHERE name = ?1",
                params![entity, now],
            )?;
        }
    }
    Ok(())
}

impl Store {
    pub(crate) fn canonical_predicate_name(
        &self,
        predicate: &str,
    ) -> Result<Option<String>, Error> {
        if self.predicate_type_id(predicate)?.is_some() {
            return Ok(Some(predicate.to_string()));
        }
        self.conn
            .query_row(
                "SELECT predicate_types.name
                   FROM predicate_alias
                   JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
                  WHERE predicate_alias.alias = ?1",
                [predicate],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }

    pub fn predicate_implies(&self, predicate: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("predicate", ancestor)?;
        let Some(predicate_name) = self.canonical_predicate_name(predicate)? else {
            return Ok(false);
        };
        let Some(ancestor_name) = self.canonical_predicate_name(ancestor)? else {
            return Ok(false);
        };
        if predicate_name == ancestor_name {
            return Ok(true);
        }
        let Some(predicate_id) = self.predicate_type_id(&predicate_name)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.predicate_type_id(&ancestor_name)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM predicate_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![predicate_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_is_a(&self, type_name: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("entity_type", type_name)?;
        validate_claim_field("entity_type", ancestor)?;
        if type_name == ancestor {
            return Ok(true);
        }
        let Some(type_id) = self.entity_type_id(type_name)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.entity_type_id(ancestor)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM entity_type_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![type_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_name(&self, entity: &str) -> Result<String, Error> {
        validate_claim_field("entity", entity)?;
        self.conn
            .query_row(
                "SELECT entity_types.name
                   FROM entities
                   JOIN entity_types ON entity_types.id = entities.type_id
                  WHERE entities.name = ?1",
                [entity],
                |row| row.get(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEntity {
                    entity: entity.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn entity_is_a_type(&self, entity: &str, ancestor: &str) -> Result<bool, Error> {
        let type_name = self.entity_type_name(entity)?;
        self.entity_type_is_a(&type_name, ancestor)
    }

    pub(crate) fn expand_predicate_filter(
        &self,
        predicates: &[&str],
    ) -> Result<HashSet<String>, Error> {
        let mut allowed = HashSet::new();
        for predicate in predicates {
            allowed.insert((*predicate).to_string());
            let Some(canonical) = self.canonical_predicate_name(predicate)? else {
                continue;
            };
            allowed.insert(canonical.clone());

            let mut stmt = self.conn.prepare(
                "SELECT child.name
                   FROM predicate_types child
                   JOIN predicate_closure closure ON closure.child_id = child.id
                   JOIN predicate_types ancestor ON ancestor.id = closure.ancestor_id
                  WHERE ancestor.name = ?1
                  ORDER BY child.id",
            )?;
            let rows = stmt.query_map([canonical.as_str()], |row| row.get::<_, String>(0))?;
            let child_names = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            for child_name in child_names {
                allowed.insert(child_name.clone());
                let mut alias_stmt = self.conn.prepare(
                    "SELECT alias
                       FROM predicate_alias
                       JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
                      WHERE predicate_types.name = ?1
                      ORDER BY alias",
                )?;
                let alias_rows =
                    alias_stmt.query_map([child_name.as_str()], |row| row.get::<_, String>(0))?;
                for alias in alias_rows {
                    allowed.insert(alias?);
                }
            }
        }
        Ok(allowed)
    }

    pub(crate) fn ensure_entity(&self, entity: &str, now: i64) -> Result<(), Error> {
        ensure_entity_on(&self.conn, entity, now)
    }

    /// ADR-0018: count of entities currently flagged `requires_review = 1`.
    /// Surfaces the silent-`Thing`-fallback queue for consolidation.
    pub fn requires_review_count(&self) -> Result<i64, Error> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entities WHERE requires_review = 1",
            [],
            |row| row.get(0),
        )?)
    }

    /// Build a user-facing diagnostic for an unknown predicate using the
    /// current runtime ontology tables.
    pub fn describe_unknown_predicate(&self, predicate: &str) -> Result<String, Error> {
        let vocabulary = self.predicate_vocabulary()?;
        let suggestion = suggest_unknown_predicate(predicate, &vocabulary.candidates);
        Ok(format_unknown_predicate(
            predicate,
            suggestion.as_ref(),
            &vocabulary.predicates,
            &vocabulary.aliases,
        ))
    }

    /// ADR-0018: resolve a predicate against `predicate_types.name` and the
    /// `predicate_alias` table.
    ///
    /// Returns `Ok(false)` if the predicate is canonical or aliased. On a
    /// `USER_ASSERTED` miss, returns `Ok(true)` so the caller can append first
    /// and then extend `predicate_types` with parent `relates_to` plus an
    /// `ontology_extension_log` record. Every other miss returns
    /// `Err(Error::UnknownPredicate)`.
    pub(crate) fn validate_ontology(
        &self,
        predicate: &str,
        provenance: Provenance,
    ) -> Result<bool, Error> {
        if self.predicate_type_id(predicate)?.is_some() {
            return Ok(false);
        }
        let alias_hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT predicate_id FROM predicate_alias WHERE alias = ?1",
                [predicate],
                |row| row.get(0),
            )
            .optional()?;
        if alias_hit.is_some() {
            return Ok(false);
        }
        match provenance {
            Provenance::UserAsserted => Ok(true),
            Provenance::Extracted | Provenance::Inferred | Provenance::Ambiguous => {
                Err(Error::UnknownPredicate {
                    name: predicate.to_string(),
                })
            }
        }
    }

    pub(crate) fn apply_ontology_extension(
        &self,
        predicate: &str,
        agent_id: &str,
        now: i64,
    ) -> Result<(), Error> {
        let parent_id = self
            .predicate_type_id("relates_to")?
            .expect("ontology bootstrap should seed relates_to");
        self.conn.execute(
            "INSERT INTO predicate_types (name, parent_id, created_via, created_at)
             VALUES (?1, ?2, 'user_assertion', ?3)",
            params![predicate, parent_id, now],
        )?;
        seed::rebuild_closure(&self.conn, "predicate_types", "predicate_closure")?;
        self.conn.execute(
            "INSERT INTO ontology_extension_log
               (predicate, parent, agent_id, created_at)
             VALUES (?1, 'relates_to', ?2, ?3)",
            params![predicate, agent_id, now],
        )?;
        Ok(())
    }

    pub(crate) fn predicate_vocabulary(&self) -> Result<PredicateVocabulary, Error> {
        let predicates =
            self.query_string_column("SELECT name FROM predicate_types ORDER BY name")?;
        let alias_rows = self.query_alias_rows()?;
        let aliases = alias_rows
            .iter()
            .map(|(alias, _canonical)| alias.clone())
            .collect::<Vec<_>>();
        let mut candidates = predicates
            .iter()
            .map(|predicate| PredicateCandidate {
                accepted: predicate.clone(),
                canonical: None,
            })
            .collect::<Vec<_>>();
        candidates.extend(
            alias_rows
                .iter()
                .map(|(alias, canonical)| PredicateCandidate {
                    accepted: alias.clone(),
                    canonical: Some(canonical.clone()),
                }),
        );
        Ok(PredicateVocabulary {
            predicates,
            aliases,
            candidates,
        })
    }

    pub(crate) fn query_string_column(&self, sql: &str) -> Result<Vec<String>, Error> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::Sqlite)
    }

    pub(crate) fn query_alias_rows(&self) -> Result<Vec<(String, String)>, Error> {
        let mut statement = self.conn.prepare(
            "SELECT predicate_alias.alias, predicate_types.name
               FROM predicate_alias
               JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
              ORDER BY predicate_alias.alias",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::Sqlite)
    }

    pub(crate) fn entity_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        entity_type_id_on(&self.conn, name)
    }

    pub(crate) fn predicate_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM predicate_types WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }
}
