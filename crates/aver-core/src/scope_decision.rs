//! Decision-model scope suggestion (ADR-0024, ADR-0021 seam).

use crate::decision::{Decision, DecisionClient, truncate_bytes};
use crate::validation::validate_scope;
use crate::{Error, Store};

/// Upper bound on `context` bytes included in the model state.
const MAX_CONTEXT_BYTES: usize = 1024;

/// Describes a scope from its ADR-0021 shape (`global`, `proj/<slug>`,
/// `proj/<slug>/branch/<name>`, `session/<id>`). Any other valid scope is
/// described by the scope string itself.
fn describe_scope(scope: &str) -> String {
    if scope == "global" {
        return "applies regardless of project or session".to_string();
    }
    if let Some(project) = scope.strip_prefix("proj/").filter(|rest| !rest.is_empty()) {
        return match project.split_once("/branch/") {
            Some((slug, branch)) => format!("specific to branch {branch} of project {slug}"),
            None => format!("specific to project {project}"),
        };
    }
    if let Some(session) = scope.strip_prefix("session/").filter(|id| !id.is_empty()) {
        return format!("only meaningful within session {session}");
    }
    scope.to_string()
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
    /// Each option is described from its scope shape. A choice whose
    /// confidence is below `min_confidence` (see
    /// [`crate::decision::Choice::meets`]) also yields `Ok(None)`. Callers
    /// normally pass [`crate::decision::DecisionConfig::min_confidence`].
    ///
    /// This is a pure read: nothing is written and the suggestion is not
    /// logged here. A caller that writes with the suggested scope uses
    /// INFERRED provenance with 0.45 confidence per ADR-0024.
    #[allow(clippy::too_many_arguments)]
    pub fn suggest_scope(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        context: &str,
        allowed: &[&str],
        client: &dyn DecisionClient,
        min_confidence: f64,
    ) -> Result<Option<String>, Error> {
        for scope in allowed {
            validate_scope(scope)?;
        }
        let state = format!(
            "Subject: {subject}. Predicate: {predicate}. Object: {object}. Context: {}",
            truncate_bytes(context, MAX_CONTEXT_BYTES)
        );
        let descriptions: Vec<String> = allowed.iter().map(|scope| describe_scope(scope)).collect();
        let description_refs: Vec<&str> = descriptions.iter().map(String::as_str).collect();
        let decision = Decision::new(&state, "Which scope does this memory belong to?", allowed)
            .with_descriptions(&description_refs);
        if decision.validate().is_err() {
            return Ok(None);
        }
        Ok(client
            .decide(&decision)
            .ok()
            .filter(|choice| choice.meets(min_confidence))
            .map(|choice| allowed[choice.index].to_string()))
    }
}
