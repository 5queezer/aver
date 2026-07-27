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
    // Starters must begin on a word boundary so "use " does not match inside
    // "because " or "reuse ".
    for starter in ["prefer ", "use ", "using "] {
        if let Some(after_verb) =
            find_word_boundary(&lower, starter).map(|pos| &lower[pos + starter.len()..])
        {
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

    dedupe_drafts(drafts)
}

/// Drop duplicate and overlapping drafts. Two rules can parse one sentence
/// ("i prefer tabs over spaces" hits both Rule 1a and Rule 4); when one
/// draft's object merely extends another's with a comparison conjunction,
/// keep the tighter extraction.
fn dedupe_drafts(drafts: Vec<CandidateClaimDraft>) -> Vec<CandidateClaimDraft> {
    let mut kept: Vec<CandidateClaimDraft> = Vec::new();
    'outer: for draft in drafts {
        let mut i = 0;
        while i < kept.len() {
            let other = &kept[i];
            let same_slot = other.event_id == draft.event_id
                && other.subject == draft.subject
                && other.predicate == draft.predicate;
            if same_slot {
                if other.object == draft.object
                    || is_conjunction_extension(&other.object, &draft.object)
                {
                    // Exact duplicate, or the kept draft is already the
                    // tighter extraction.
                    continue 'outer;
                }
                if is_conjunction_extension(&draft.object, &other.object) {
                    // The new draft is the tighter extraction; replace the
                    // kept one and keep scanning.
                    kept.remove(i);
                    continue;
                }
            }
            i += 1;
        }
        kept.push(draft);
    }
    kept
}

/// True when `longer` is `shorter` plus a comparison conjunction, e.g.
/// "tabs over spaces" extending "tabs" with " over ...".
fn is_conjunction_extension(shorter: &str, longer: &str) -> bool {
    longer.strip_prefix(shorter).is_some_and(|rest| {
        [" for ", " over ", " instead "]
            .iter()
            .any(|conjunction| rest.starts_with(conjunction))
    })
}

/// Find the first occurrence of `needle` starting on a word boundary (start
/// of the string or a non-alphanumeric predecessor).
fn find_word_boundary(haystack: &str, needle: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs = start + pos;
        let on_boundary = abs == 0
            || haystack[..abs]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_ascii_alphanumeric());
        if on_boundary {
            return Some(abs);
        }
        start = abs + 1;
    }
    None
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
