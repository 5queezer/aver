//! Decision-model predicate normalization (ADR-0024).

use crate::decision::{Decision, DecisionClient, MAX_OPTIONS};
use crate::{
    Error, PredicateCandidate, PredicateSuggestion, Store, edit_distance, normalize_predicate_name,
    semantic_predicate_hint, suggest_unknown_predicate,
};

const NONE_OPTION: &str = "none";

impl Store {
    /// Map an unknown predicate name onto the accepted vocabulary with a
    /// closed-choice decision model.
    ///
    /// Returns `Ok(None)` when the predicate is already canonical or an
    /// accepted alias, or when the model answers `none`. Any
    /// [`crate::decision::DecisionError`] falls back to the deterministic
    /// `suggest_unknown_predicate` result.
    ///
    /// This is a pure read. Callers that write a claim with the suggested
    /// predicate are responsible for recording it as `INFERRED` with
    /// confidence 0.45.
    pub fn normalize_predicate(
        &self,
        predicate: &str,
        client: &dyn DecisionClient,
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
        let mut options: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.accepted.as_str())
            .collect();
        options.push(NONE_OPTION);

        let state = format!("predicate: {predicate}\nnormalized: {normalized}");
        let decision = Decision {
            state: &state,
            question: "Which canonical predicate does this input mean?",
            options: &options,
        };
        match client.decide(&decision) {
            Ok(choice) => Ok(candidates
                .get(choice.index)
                .map(|candidate| PredicateSuggestion {
                    accepted: candidate.accepted.clone(),
                    canonical: candidate.canonical.clone(),
                })),
            Err(_) => Ok(suggest_unknown_predicate(predicate, &vocabulary.candidates)),
        }
    }
}

/// Deduplicates candidates by accepted name and sorts them for a stable
/// prompt. If more than `MAX_OPTIONS - 1` remain (one slot is reserved for
/// `none`), keeps the best matches ranked like `suggest_unknown_predicate`:
/// the semantic-hint target first, then smallest edit distance, then shorter
/// name, then alphabetical; the survivors are re-sorted alphabetically.
fn narrow_candidates(
    normalized: &str,
    mut candidates: Vec<PredicateCandidate>,
) -> Vec<PredicateCandidate> {
    candidates.retain(|candidate| candidate.accepted != NONE_OPTION);
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
