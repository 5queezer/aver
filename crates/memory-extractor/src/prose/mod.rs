//! Structured-output parsing for prose extraction (ADR-0007).

use crate::{Error, ExtractedFact};

#[derive(serde::Deserialize)]
struct ProseExtraction {
    facts: Vec<ExtractedFact>,
}

pub fn parse_prose_facts(output: &str) -> Result<Vec<ExtractedFact>, Error> {
    Ok(serde_json::from_str::<ProseExtraction>(output)?.facts)
}
