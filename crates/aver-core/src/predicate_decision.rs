//! Decision-model predicate normalization (ADR-0024).

use crate::decision::{Decision, DecisionClient, MAX_OPTIONS};
use crate::ontology::{
    PredicateCandidate, PredicateSuggestion, edit_distance, normalize_predicate_name,
    semantic_predicate_hint, suggest_unknown_predicate,
};
use crate::{Error, Store};

/// Name of the trailing "no match" option. A vocabulary predicate with the
/// same name makes it ambiguous, so [`none_option`] then picks another.
const NONE_OPTION: &str = "none";
const NONE_OPTION_FALLBACK: &str = "none_of_the_above";
const NONE_DESCRIPTION: &str = "no listed predicate fits this input";

/// Instructions verified live against `tev1:4b` on 2026-10-24 (with the
/// default `none` sentinel).
fn question(none: &str) -> String {
    format!(
        "Pick the canonical predicate whose meaning matches the input predicate. \
         Pick {none} only if no option has the same meaning."
    )
}

/// The "no match" option name for this call: `none`, unless the vocabulary
/// already uses it, then `none_of_the_above`, extended with `_` until it
/// collides with no candidate.
fn none_option(candidates: &[PredicateCandidate]) -> String {
    let taken = |name: &str| candidates.iter().any(|c| c.accepted == name);
    if !taken(NONE_OPTION) {
        return NONE_OPTION.to_string();
    }
    let mut name = NONE_OPTION_FALLBACK.to_string();
    while taken(&name) {
        name.push('_');
    }
    name
}

/// One-line meanings for the seeded predicates that `semantic_predicate_hint`
/// (lib.rs) maps synonyms onto; each line paraphrases that synonym group.
/// The `depends_on` line is the description verified live against `tev1:4b`
/// on 2026-10-24. Other predicates are described by name and aliases only.
const PREDICATE_MEANINGS: [(&str, &str); 6] = [
    (
        "depends_on",
        "subject requires, needs, or relies on the object",
    ),
    ("fixes", "subject fixes, resolves, or repairs the object"),
    ("owns", "subject owns or is the owner of the object"),
    (
        "prefers",
        "subject favors, likes, or prioritizes the object",
    ),
    (
        "relates_to",
        "subject is about, related to, or associated with the object",
    ),
    ("tests", "subject tests or validates the object"),
];

impl Store {
    /// Map an unknown predicate name onto the accepted vocabulary with a
    /// closed-choice decision model.
    ///
    /// Each option is described by its name, its aliases (or the predicate
    /// it is an alias of) and, for a few seeded predicates, a one-line
    /// meaning. `example` is an optional sentence using the predicate (for
    /// instance built from the claim's subject and object) that is shown to
    /// the model.
    ///
    /// Returns `Ok(None)` when the predicate is already canonical or an
    /// accepted alias, or when the model answers the trailing "no match"
    /// option (`none`, or `none_of_the_above` when `none` is itself a
    /// vocabulary predicate). Any
    /// [`crate::decision::DecisionError`], and any choice whose confidence is
    /// below `min_confidence` (see [`crate::decision::Choice::meets`]), falls
    /// back to the deterministic `suggest_unknown_predicate` result. Callers
    /// normally pass [`crate::decision::DecisionConfig::min_confidence`].
    ///
    /// This is a pure read. Callers that write a claim with the suggested
    /// predicate are responsible for recording it as `INFERRED` with
    /// confidence 0.45.
    pub fn normalize_predicate(
        &self,
        predicate: &str,
        example: Option<&str>,
        client: &dyn DecisionClient,
        min_confidence: f64,
    ) -> Result<Option<PredicateSuggestion>, Error> {
        let vocabulary = self.predicate_vocabulary()?;
        if vocabulary
            .candidates
            .iter()
            .any(|candidate| candidate.accepted == predicate)
        {
            return Ok(None);
        }

        let normalized = normalize_predicate_name(predicate);
        let candidates = narrow_candidates(&normalized, vocabulary.candidates.clone());
        let none = none_option(&candidates);
        let mut options: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.accepted.as_str())
            .collect();
        options.push(&none);
        let descriptions: Vec<String> = candidates
            .iter()
            .map(|candidate| describe(candidate, &vocabulary.candidates))
            .collect();
        let mut description_refs: Vec<&str> = descriptions.iter().map(String::as_str).collect();
        description_refs.push(NONE_DESCRIPTION);

        let mut state = format!("predicate: {predicate}\nnormalized: {normalized}");
        if let Some(example) = example {
            state = format!("{state}\nexample: {example}");
        }
        let instructions = question(&none);
        let decision =
            Decision::new(&state, &instructions, &options).with_descriptions(&description_refs);
        match client.decide(&decision) {
            Ok(choice) if choice.meets(min_confidence) => {
                Ok(candidates
                    .get(choice.index)
                    .map(|candidate| PredicateSuggestion {
                        accepted: candidate.accepted.clone(),
                        canonical: candidate.canonical.clone(),
                    }))
            }
            _ => Ok(suggest_unknown_predicate(predicate, &vocabulary.candidates)),
        }
    }
}

/// `"depends_on (also: needs, requires): <meaning>"` for a canonical
/// predicate, `"needs (alias of depends_on): <meaning>"` for an alias. The
/// meaning comes from [`PREDICATE_MEANINGS`] and is omitted when absent.
fn describe(candidate: &PredicateCandidate, vocabulary: &[PredicateCandidate]) -> String {
    let description = match &candidate.canonical {
        Some(canonical) => format!("{} (alias of {canonical})", candidate.accepted),
        None => {
            let aliases: Vec<&str> = vocabulary
                .iter()
                .filter(|other| other.canonical.as_deref() == Some(candidate.accepted.as_str()))
                .map(|other| other.accepted.as_str())
                .collect();
            if aliases.is_empty() {
                candidate.accepted.clone()
            } else {
                format!("{} (also: {})", candidate.accepted, aliases.join(", "))
            }
        }
    };
    let canonical = candidate
        .canonical
        .as_deref()
        .unwrap_or(&candidate.accepted);
    match PREDICATE_MEANINGS
        .iter()
        .find(|(name, _)| *name == canonical)
    {
        Some((_, meaning)) => format!("{description}: {meaning}"),
        None => description,
    }
}

/// Deduplicates candidates by accepted name and sorts them for a stable
/// prompt. If more than `MAX_OPTIONS - 1` remain (one slot is reserved for
/// the "no match" option), keeps the best matches ranked like `suggest_unknown_predicate`:
/// the semantic-hint target first, then smallest edit distance, then shorter
/// name, then alphabetical; the survivors are re-sorted alphabetically.
fn narrow_candidates(
    normalized: &str,
    mut candidates: Vec<PredicateCandidate>,
) -> Vec<PredicateCandidate> {
    candidates.sort_by(|a, b| a.accepted.cmp(&b.accepted));
    candidates.dedup_by(|a, b| a.accepted == b.accepted);
    if candidates.len() > MAX_OPTIONS - 1 {
        let hint = semantic_predicate_hint(normalized);
        candidates.sort_by_cached_key(|candidate| {
            (
                Some(candidate.accepted.as_str()) != hint,
                edit_distance(normalized, &normalize_predicate_name(&candidate.accepted)),
                candidate.accepted.len(),
                candidate.accepted.clone(),
            )
        });
        candidates.truncate(MAX_OPTIONS - 1);
        candidates.sort_by(|a, b| a.accepted.cmp(&b.accepted));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(accepted: &str, canonical: Option<&str>) -> PredicateCandidate {
        PredicateCandidate {
            accepted: accepted.to_string(),
            canonical: canonical.map(str::to_string),
        }
    }

    #[test]
    fn describe_lists_aliases_and_seeded_meaning() {
        let vocabulary = [
            candidate("depends_on", None),
            candidate("needs", Some("depends_on")),
            candidate("requires", Some("depends_on")),
            candidate("emits", None),
        ];
        assert_eq!(
            describe(&vocabulary[0], &vocabulary),
            "depends_on (also: needs, requires): subject requires, needs, or relies on the object"
        );
        assert_eq!(
            describe(&vocabulary[1], &vocabulary),
            "needs (alias of depends_on): subject requires, needs, or relies on the object"
        );
        assert_eq!(describe(&vocabulary[3], &vocabulary), "emits");
    }

    #[test]
    fn none_option_avoids_vocabulary_names() {
        assert_eq!(none_option(&[candidate("owns", None)]), "none");
        assert_eq!(none_option(&[candidate("none", None)]), "none_of_the_above");
        assert_eq!(
            none_option(&[
                candidate("none", None),
                candidate("none_of_the_above", None),
            ]),
            "none_of_the_above_"
        );
    }
}
