//! Decision-model scope suggestion (ADR-0024, ADR-0021 seam).

use crate::decision::{Decision, DecisionClient};
use crate::validation::validate_scope;
use crate::{Error, Store};

/// Upper bound on `context` bytes included in the model state.
const MAX_CONTEXT_BYTES: usize = 1024;

fn truncate_context(context: &str) -> &str {
    if context.len() <= MAX_CONTEXT_BYTES {
        return context;
    }
    let mut end = MAX_CONTEXT_BYTES;
    while !context.is_char_boundary(end) {
        end -= 1;
    }
    &context[..end]
}

impl Store {
    /// Asks a decision model which of the caller's `allowed` scopes a memory
    /// belongs to. `context` is the source event payload or extractor file
    /// path; it is truncated before being shown to the model.
    ///
    /// Every entry of `allowed` must pass scope validation (otherwise the
    /// scope error is returned and the client is not called). Fewer than two
    /// options, or any `DecisionError`, yields `Ok(None)` so the caller keeps
    /// its current default scope.
    ///
    /// This is a pure read: nothing is written and the suggestion is not
    /// logged here. A caller that writes with the suggested scope uses
    /// INFERRED provenance with 0.45 confidence per ADR-0024.
    pub fn suggest_scope(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        context: &str,
        allowed: &[&str],
        client: &dyn DecisionClient,
    ) -> Result<Option<String>, Error> {
        for scope in allowed {
            validate_scope(scope)?;
        }
        let state = format!(
            "Subject: {subject}. Predicate: {predicate}. Object: {object}. Context: {}",
            truncate_context(context)
        );
        let decision = Decision {
            state: &state,
            question: "Which scope does this memory belong to?",
            options: allowed,
        };
        if decision.validate().is_err() {
            return Ok(None);
        }
        Ok(client
            .decide(&decision)
            .ok()
            .map(|choice| allowed[choice.index].to_string()))
    }
}
