//! Deterministic chat-event claim extraction rules.
//!
//! Parses EpisodicEvent payloads with simple keyword/substring rules and
//! produces CandidateClaimDraft values without requiring an LLM or regex crate.

use crate::{CandidateClaimDraft, ClaimExtractor, EpisodicEvent, Error};

/// A deterministic rule-based extractor for chat session payloads.
///
/// Rules (all case-insensitive):
/// 1. "i prefer X" / "i like X"            -> ("User", "prefers", X)
/// 2. "i decided to X" / "i chose to X" /
///    "we decided to X"                     -> ("User", "decides", X)
/// 3. "the project uses X" / "is X" /
///    "has X"                               -> ("Project", <verb>, X)
/// 4. "prefer X for|over|instead" /
///    "use X for|over|instead" /
///    "using X for|over|instead"            -> ("User", "prefers", X)
/// 5. "we use X" / "we're using X"         -> ("User", "uses", X)
pub struct ChatEventExtractor;

impl ClaimExtractor for ChatEventExtractor {
    fn extract(&self, events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error> {
        let mut drafts = Vec::new();
        for event in events {
            for draft in extract_from_payload(event.id, &event.payload) {
                drafts.push(draft);
            }
        }
        Ok(drafts)
    }
}

fn extract_from_payload(event_id: i64, payload: &str) -> Vec<CandidateClaimDraft> {
    let lower = payload.to_lowercase();
    let mut drafts = Vec::new();

    // Rule 1a: "i prefer X"
    if let Some(rest) = lower.strip_prefix("i prefer ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "prefers", object));
        }
    }
    // Rule 1b: "i like X"
    if let Some(rest) = lower.strip_prefix("i like ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "prefers", object));
        }
    }

    // Rule 2a: "i decided to X"
    if let Some(rest) = lower.strip_prefix("i decided to ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "decides", object));
        }
    }
    // Rule 2b: "i chose to X"
    if let Some(rest) = lower.strip_prefix("i chose to ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "decides", object));
        }
    }
    // Rule 2c: "we decided to X"
    if let Some(rest) = lower.strip_prefix("we decided to ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "decides", object));
        }
    }

    // Rule 3: "the project uses|is|has X"
    for (verb_lower, predicate) in [("uses", "uses"), ("is", "is"), ("has", "has")] {
        let needle = format!("the project {verb_lower} ");
        if let Some(rest) = lower.find(&needle).map(|pos| &lower[pos + needle.len()..]) {
            let object = trim_trailing(rest);
            if !object.is_empty() {
                drafts.push(draft(event_id, "Project", predicate, object));
            }
        }
    }

    // Rule 4: "prefer X for|over|instead" / "use X for|over|instead" / "using X for|over|instead"
    for starter in ["prefer ", "use ", "using "] {
        if let Some(after_verb) = lower.find(starter).map(|pos| &lower[pos + starter.len()..]) {
            // find first occurrence of "for ", "over ", or "instead"
            let object = if let Some(pos) = find_first(after_verb, &["for ", "over ", "instead"]) {
                trim_trailing(&after_verb[..pos])
            } else {
                ""
            };
            if !object.is_empty() {
                drafts.push(draft(event_id, "User", "prefers", object));
            }
        }
    }

    // Rule 5a: "we use X"
    if let Some(rest) = lower.strip_prefix("we use ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "uses", object));
        }
    }
    // Rule 5b: "we're using X"
    if let Some(rest) = lower.strip_prefix("we're using ") {
        let object = trim_trailing(rest);
        if !object.is_empty() {
            drafts.push(draft(event_id, "User", "uses", object));
        }
    }

    drafts
}

fn draft(event_id: i64, subject: &str, predicate: &str, object: &str) -> CandidateClaimDraft {
    CandidateClaimDraft {
        event_id,
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.trim().to_string(),
    }
}

fn trim_trailing(s: &str) -> &str {
    s.trim_end_matches(['.', '!', '?', ',', ';', ':', ' '])
}

/// Find the earliest position of any needle in the haystack.
fn find_first(haystack: &str, needles: &[&str]) -> Option<usize> {
    needles
        .iter()
        .filter_map(|needle| haystack.find(needle))
        .min()
}
