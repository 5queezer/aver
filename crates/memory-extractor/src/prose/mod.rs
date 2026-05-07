//! Structured-output parsing for prose extraction (ADR-0007).

use crate::{Error, ExtractedFact};

#[derive(serde::Deserialize)]
struct ProseExtraction {
    facts: Vec<ExtractedFact>,
}

pub fn parse_prose_facts(output: &str) -> Result<Vec<ExtractedFact>, Error> {
    let extraction = serde_json::from_str::<ProseExtraction>(output)?;
    for fact in &extraction.facts {
        if fact.subject.trim().is_empty() {
            return Err(Error::InvalidFact("subject"));
        }
        if fact.predicate.trim().is_empty() {
            return Err(Error::InvalidFact("predicate"));
        }
        if fact.object.trim().is_empty() {
            return Err(Error::InvalidFact("object"));
        }
    }
    Ok(extraction.facts)
}
