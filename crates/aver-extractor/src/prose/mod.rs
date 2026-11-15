//! Structured-output parsing for prose extraction (ADR-0007).

use crate::{Error, ExtractedFact, validate_facts};

#[derive(serde::Deserialize)]
struct ProseExtraction {
    facts: Vec<ExtractedFact>,
}

pub fn parse_prose_facts(output: &str) -> Result<Vec<ExtractedFact>, Error> {
    let extraction = serde_json::from_str::<ProseExtraction>(output)?;
    validate_facts(&extraction.facts)?;
    Ok(extraction.facts)
}
