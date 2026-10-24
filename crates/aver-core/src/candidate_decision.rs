//! ADR-0024 decision-model seams for the ADR-0014 pipeline: candidate
//! triage (`promote`, `reject`, `hold`) and the extraction trigger
//! (`extract`, `skip`). The model only picks an option enumerated here;
//! replay reads the recorded choice and never calls a client.

use serde::Serialize;

use crate::decision::{Decision, DecisionClient, DecisionError};
use crate::{Error, ExtractionDecision, ExtractionTriggerReason, Provenance, Store};

const TRIAGE_QUESTION: &str = "What should happen to this staged claim?";
const TRIAGE_OPTIONS: [&str; 3] = ["promote", "reject", "hold"];
const TRIGGER_QUESTION: &str = "Should this event batch run extraction now?";
const TRIGGER_OPTIONS: [&str; 2] = ["extract", "skip"];

/// ADR-0003 policy row "single-source LLM inference".
const DECISION_PROVENANCE: Provenance = Provenance::Inferred;
const DECISION_CONFIDENCE: f64 = 0.45;

/// Per-payload and total byte bounds for the state handed to the model.
const PAYLOAD_EXCERPT_BYTES: usize = 512;
const STATE_MAX_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateTriage {
    Promote,
    Reject,
    Hold,
}

/// ADR-0024 `decision` field on promote/reject candidate log lines:
/// `{"model": "...", "choice": "promote" | "reject"}`.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct DecisionLogField<'a> {
    pub(crate) model: &'a str,
    pub(crate) choice: &'a str,
}

/// Validates the optional `decision` field of a candidate lifecycle line.
/// Absent is fine (manual promotion/rejection and pre-ADR-0024 lines); when
/// present it must name a model and the choice matching the line's kind.
pub(crate) fn replay_decision(
    value: &serde_json::Value,
    expected_choice: &str,
    path: &str,
    lineno: usize,
) -> Result<(), Error> {
    let Some(raw) = value.get("decision") else {
        return Ok(());
    };
    let model = raw.get("model").and_then(|v| v.as_str());
    let choice = raw.get("choice").and_then(|v| v.as_str());
    match (model, choice) {
        (Some(model), Some(choice)) if !model.trim().is_empty() && choice == expected_choice => {
            Ok(())
        }
        _ => Err(Error::ReplayMalformed {
            path: path.to_string(),
            line: lineno,
            detail: format!(
                "'decision' must be {{\"model\": <non-empty string>, \"choice\": \"{expected_choice}\"}}, got {raw}"
            ),
        }),
    }
}

/// Truncates to at most `max` bytes on a char boundary.
fn excerpt(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

impl Store {
    /// ADR-0024 candidate triage. Asks `client` whether a PENDING candidate
    /// should be promoted, rejected or held. Promotion records provenance
    /// `INFERRED` / confidence `0.45`; rejection records reason
    /// `decision:<model>`; both log the choice. `Hold` and any client error
    /// write nothing.
    pub fn triage_candidate_claim(
        &self,
        candidate_id: i64,
        client: &dyn DecisionClient,
    ) -> Result<CandidateTriage, Error> {
        let candidate = self.get_candidate_claim(candidate_id)?;
        if candidate.status != "PENDING" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        // Source payloads passed the privacy gate at record time.
        let event = self.get_event(candidate.event_id)?;
        let state = serde_json::json!({
            "candidate": {
                "subject": candidate.subject,
                "predicate": candidate.predicate,
                "object": candidate.object,
            },
            "scope": candidate.scope,
            "source_events": [{
                "id": event.id,
                "kind": event.kind,
                "payload": excerpt(&event.payload, STATE_MAX_BYTES),
            }],
        })
        .to_string();
        let choice = client.decide(&Decision {
            state: &state,
            question: TRIAGE_QUESTION,
            options: &TRIAGE_OPTIONS,
        })?;
        let triage = match choice.index {
            0 => CandidateTriage::Promote,
            1 => CandidateTriage::Reject,
            2 => CandidateTriage::Hold,
            index => {
                return Err(DecisionError::InvalidChoice {
                    raw: index.to_string(),
                }
                .into());
            }
        };
        let decision = DecisionLogField {
            model: &choice.model,
            choice: TRIAGE_OPTIONS[choice.index],
        };
        match triage {
            CandidateTriage::Promote => {
                self.promote_candidate_claim_inner(
                    candidate_id,
                    Some((DECISION_PROVENANCE, DECISION_CONFIDENCE)),
                    Some(decision),
                )?;
            }
            CandidateTriage::Reject => {
                let reason = format!("decision:{}", choice.model);
                self.reject_candidate_claim_inner(candidate_id, &reason, Some(decision))?;
            }
            CandidateTriage::Hold => {}
        }
        Ok(triage)
    }

    /// ADR-0024 extraction trigger. Runs the deterministic
    /// [`Store::extraction_decision`]; when it found nothing beyond the
    /// coverage-gap signal (which alone does not justify extraction, see
    /// [`Store::should_extract_memories`]) but the session has undigested
    /// events, asks `client` whether to extract now. `extract` adds
    /// [`ExtractionTriggerReason::DecisionModel`]; `skip` or any client
    /// error returns the deterministic decision. Nothing is logged.
    pub fn extraction_decision_with_client(
        &self,
        session_id: &str,
        event_threshold: usize,
        observation_token_threshold: Option<usize>,
        client: &dyn DecisionClient,
    ) -> Result<ExtractionDecision, Error> {
        let mut decision =
            self.extraction_decision(session_id, event_threshold, observation_token_threshold)?;
        if decision
            .reasons
            .iter()
            .any(|reason| *reason != ExtractionTriggerReason::UncoveredCoverageGap)
        {
            return Ok(decision);
        }
        let uncovered = self.observation_coverage(session_id)?.uncovered_event_ids;
        if uncovered.is_empty() {
            return Ok(decision);
        }

        let mut state = format!("undigested_events: {}\n", uncovered.len());
        for event_id in &uncovered {
            let event = self.get_event(*event_id)?;
            let line = format!(
                "- [{}] {}\n",
                event.kind,
                excerpt(&event.payload, PAYLOAD_EXCERPT_BYTES)
            );
            if state.len() + line.len() > STATE_MAX_BYTES {
                break;
            }
            state.push_str(&line);
        }

        let extract = client
            .decide(&Decision {
                state: &state,
                question: TRIGGER_QUESTION,
                options: &TRIGGER_OPTIONS,
            })
            .is_ok_and(|choice| choice.index == 0);
        if extract {
            decision
                .reasons
                .push(ExtractionTriggerReason::DecisionModel);
            decision.should_extract = true;
        }
        Ok(decision)
    }
}
