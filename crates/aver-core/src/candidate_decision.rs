//! ADR-0024 decision-model seams for the ADR-0014 pipeline: candidate
//! triage (`promote`, `reject`, `hold`) and the extraction trigger
//! (`extract`, `skip`). The model only picks an option enumerated here;
//! replay reads the recorded choice and never calls a client.

use std::collections::HashSet;

use serde::Serialize;

use crate::decision::{Decision, DecisionClient, DecisionError, truncate_bytes};
use crate::{Error, ExtractionDecision, ExtractionTriggerReason, Provenance, Store};

const TRIAGE_QUESTION: &str =
    "What should happen to this staged claim? Judge it only against its source text.";
const TRIAGE_OPTIONS: [&str; 3] = ["promote", "reject", "hold"];
/// Criteria descriptions verified live against `tev1:4b` on 2026-10-24.
const TRIAGE_DESCRIPTIONS: [&str; 3] = [
    "the claim is a specific, durable fact or preference that the source text clearly supports",
    "the claim is wrong, trivial, conversational filler, or not supported by the source text",
    "the claim may be useful but the source text is too ambiguous to decide",
];
const TRIGGER_QUESTION: &str =
    "Should these undigested events be run through memory extraction now?";
const TRIGGER_OPTIONS: [&str; 2] = ["extract", "skip"];
const TRIGGER_DESCRIPTIONS: [&str; 2] = [
    "the events state durable facts, decisions, preferences, or dependencies worth remembering",
    "the events are greetings, chatter, routine tool output, or too little to extract from yet",
];

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

// Field order is deliberate: the model is order-sensitive (verified 2026-10-24).
#[derive(Serialize)]
struct TriageState<'a> {
    candidate: TriageCandidate<'a>,
    scope: &'a str,
    source_events: [TriageEvent<'a>; 1],
}

// Field order is deliberate: the model is order-sensitive (verified 2026-10-24).
#[derive(Serialize)]
struct TriageCandidate<'a> {
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
}

// Field order is deliberate: the model is order-sensitive (verified 2026-10-24).
#[derive(Serialize)]
struct TriageEvent<'a> {
    id: i64,
    kind: &'a str,
    payload: &'a str,
}

/// ADR-0024 `decision` field on promote/reject candidate log lines:
/// `{"model": "...", "choice": "promote" | "reject"}`.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct DecisionLogField<'a> {
    pub(crate) model: &'a str,
    pub(crate) choice: &'a str,
}

impl Store {
    /// ADR-0024 candidate triage. Asks `client` whether a PENDING candidate
    /// should be promoted, rejected or held. Promotion records provenance
    /// `INFERRED` / confidence `0.45`; rejection records reason
    /// `decision:<model>`; both log the choice. `Hold` and any client error
    /// write nothing.
    ///
    /// A `promote` or `reject` choice whose confidence is below
    /// `min_confidence` (see [`crate::decision::Choice::meets`]) is
    /// downgraded to `Hold`. Callers normally pass
    /// [`crate::decision::DecisionConfig::min_confidence`].
    pub fn triage_candidate_claim(
        &self,
        candidate_id: i64,
        client: &dyn DecisionClient,
        min_confidence: f64,
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
        let state = serde_json::to_string(&TriageState {
            candidate: TriageCandidate {
                subject: &candidate.subject,
                predicate: &candidate.predicate,
                object: &candidate.object,
            },
            scope: &candidate.scope,
            source_events: [TriageEvent {
                id: event.id,
                kind: &event.kind,
                payload: truncate_bytes(&event.payload, STATE_MAX_BYTES),
            }],
        })
        .expect("triage state serializes");
        let choice = client.decide(
            &Decision::new(&state, TRIAGE_QUESTION, &TRIAGE_OPTIONS)
                .with_descriptions(&TRIAGE_DESCRIPTIONS),
        )?;
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
        let triage = if choice.meets(min_confidence) {
            triage
        } else {
            CandidateTriage::Hold
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
    /// [`ExtractionTriggerReason::DecisionModel`]; `skip`, an `extract`
    /// below `min_confidence` (see [`crate::decision::Choice::meets`]) or any
    /// client error returns the deterministic decision. Nothing is logged.
    pub fn extraction_decision_with_client(
        &self,
        session_id: &str,
        event_threshold: usize,
        observation_token_threshold: Option<usize>,
        client: &dyn DecisionClient,
        min_confidence: f64,
    ) -> Result<ExtractionDecision, Error> {
        let mut decision =
            self.extraction_decision(session_id, event_threshold, observation_token_threshold)?;
        // The model is asked only when the coverage gap is the sole reason.
        // `extraction_decision` already computed coverage: without the gap
        // reason the session has no undigested events, so no second pass.
        if decision.reasons != [ExtractionTriggerReason::UncoveredCoverageGap] {
            return Ok(decision);
        }
        // One observation and one event listing; uncovered events are
        // filtered in memory instead of re-read one by one.
        let covered: HashSet<i64> = self
            .list_observations_for_session(session_id)?
            .into_iter()
            .flat_map(|observation| observation.source_event_ids)
            .collect();
        let uncovered: Vec<_> = self
            .list_events_for_session(session_id)?
            .into_iter()
            .filter(|event| !covered.contains(&event.id))
            .collect();
        if uncovered.is_empty() {
            return Ok(decision);
        }

        let mut state = format!("undigested_events: {}\n", uncovered.len());
        for event in &uncovered {
            let line = format!(
                "- [{}] {}\n",
                event.kind,
                truncate_bytes(&event.payload, PAYLOAD_EXCERPT_BYTES)
            );
            if state.len() + line.len() > STATE_MAX_BYTES {
                break;
            }
            state.push_str(&line);
        }

        let extract = client
            .decide(
                &Decision::new(&state, TRIGGER_QUESTION, &TRIGGER_OPTIONS)
                    .with_descriptions(&TRIGGER_DESCRIPTIONS),
            )
            .is_ok_and(|choice| choice.index == 0 && choice.meets(min_confidence));
        if extract {
            decision
                .reasons
                .push(ExtractionTriggerReason::DecisionModel);
            decision.should_extract = true;
        }
        Ok(decision)
    }
}
