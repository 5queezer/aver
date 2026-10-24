//! Closed-choice decision models (ADR-0024).
//!
//! A decision client reads a state, a question and 2..=24 caller-enumerated
//! options and returns the index of exactly one option. It never produces
//! free text, so no model output can enter memory except as a choice Aver
//! already listed.

use std::collections::VecDeque;
use std::sync::Mutex;

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ollama http: {0}")]
    Http(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("model answer does not map to an option: {raw:?}")]
    InvalidChoice { raw: String },
    #[error("decision needs at least {MIN_OPTIONS} options, got {count}")]
    TooFewOptions { count: usize },
    #[error("decision allows at most {MAX_OPTIONS} options, got {count}")]
    TooManyOptions { count: usize },
    #[error("decision options must be non-empty and unique, got {option:?}")]
    DuplicateOption { option: String },
    #[error("decision model unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision<'a> {
    pub state: &'a str,
    pub question: &'a str,
    pub options: &'a [&'a str],
}

impl Decision<'_> {
    pub fn validate(&self) -> Result<(), DecisionError> {
        let count = self.options.len();
        if count < MIN_OPTIONS {
            return Err(DecisionError::TooFewOptions { count });
        }
        if count > MAX_OPTIONS {
            return Err(DecisionError::TooManyOptions { count });
        }
        // systemone keys options by string: duplicates would silently collapse.
        let mut seen = std::collections::HashSet::new();
        for &option in self.options {
            if option.is_empty() || !seen.insert(option) {
                return Err(DecisionError::DuplicateOption {
                    option: option.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// Always a valid index into the decision's `options`.
    pub index: usize,
    /// Model-reported confidence for the chosen option, when provided.
    pub confidence: Option<f64>,
    pub model: String,
}

pub trait DecisionClient {
    fn decide(&self, decision: &Decision<'_>) -> Result<Choice, DecisionError>;
}

/// Client for deployments without a decision model; callers fall back to
/// their deterministic behaviour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoopDecisionClient;

impl DecisionClient for NoopDecisionClient {
    fn decide(&self, _decision: &Decision<'_>) -> Result<Choice, DecisionError> {
        Err(DecisionError::Unavailable)
    }
}

/// Owned copy of a [`Decision`] received by [`MockDecisionClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedDecision {
    pub state: String,
    pub question: String,
    pub options: Vec<String>,
}

/// Scripted client for offline tests. Answers are consumed in order; an
/// exhausted script answers [`DecisionError::Unavailable`].
#[derive(Debug, Default)]
pub struct MockDecisionClient {
    answers: Mutex<VecDeque<Result<usize, DecisionError>>>,
    received: Mutex<Vec<RecordedDecision>>,
}

impl MockDecisionClient {
    pub const MODEL: &'static str = "mock";

    pub fn new(answers: Vec<Result<usize, DecisionError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            received: Mutex::new(Vec::new()),
        }
    }

    pub fn with_indices(indices: Vec<usize>) -> Self {
        Self::new(indices.into_iter().map(Ok).collect())
    }

    pub fn received(&self) -> Vec<RecordedDecision> {
        self.received
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl DecisionClient for MockDecisionClient {
    fn decide(&self, decision: &Decision<'_>) -> Result<Choice, DecisionError> {
        self.received
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RecordedDecision {
                state: decision.state.to_string(),
                question: decision.question.to_string(),
                options: decision.options.iter().map(|o| o.to_string()).collect(),
            });
        decision.validate()?;
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or(Err(DecisionError::Unavailable))?;
        if answer >= decision.options.len() {
            return Err(DecisionError::InvalidChoice {
                raw: answer.to_string(),
            });
        }
        Ok(Choice {
            index: answer,
            confidence: None,
            model: Self::MODEL.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionConfig {
    pub base_url: String,
    pub model: String,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            model: "tev1:4b".to_string(),
        }
    }
}

impl DecisionConfig {
    /// Reads `AVER_DECISION_URL` and `AVER_DECISION_MODEL`; unset or blank
    /// values keep the default.
    pub fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let default = Self::default();
        Self {
            base_url: var("AVER_DECISION_URL").unwrap_or(default.base_url),
            model: var("AVER_DECISION_MODEL").unwrap_or(default.model),
        }
    }
}

/// Name of the single `choice` question sent to `/v1/systemone`.
#[cfg(feature = "ollama")]
const QUESTION_NAME: &str = "decision";

#[cfg(feature = "ollama")]
#[derive(Debug, Clone)]
pub struct OllamaDecisionClient {
    base_url: String,
    model: String,
}

#[cfg(feature = "ollama")]
impl OllamaDecisionClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn systemone_url(&self) -> String {
        format!("{}/v1/systemone", self.base_url)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Jev-style request: one `choice` question whose `criteria` maps each
    /// option key to its description. Aver options are their own key and
    /// description.
    pub fn build_request(&self, decision: &Decision<'_>) -> serde_json::Value {
        let criteria: serde_json::Map<String, serde_json::Value> = decision
            .options
            .iter()
            .map(|&option| (option.to_string(), serde_json::Value::from(option)))
            .collect();
        serde_json::json!({
            "model": self.model,
            "state": decision.state,
            "questions": {
                QUESTION_NAME: {
                    "type": "choice",
                    "instructions": decision.question,
                    "criteria": criteria,
                },
            },
        })
    }

    /// Reads the first answer of a systemone response. This shape was
    /// verified live against `tev1:4b` on 2026-10-24. `choice` may be an
    /// option key (matched by key) or a zero-based index (range-checked);
    /// `answers` may be keyed by question name or a list in question order.
    pub fn parse_response(
        body: &str,
        options: &[&str],
    ) -> Result<(usize, Option<f64>), DecisionError> {
        let response: serde_json::Value = serde_json::from_str(body)?;
        let answer = match response.get("answers") {
            Some(serde_json::Value::Object(map)) => map.get(QUESTION_NAME).or(map.values().next()),
            Some(serde_json::Value::Array(list)) => list.first(),
            _ => None,
        };
        let raw = answer.and_then(|a| a.get("choice"));
        let invalid = || DecisionError::InvalidChoice {
            raw: raw.map_or_else(|| "<missing>".to_string(), |value| value.to_string()),
        };
        let index = match raw.ok_or_else(invalid)? {
            serde_json::Value::String(key) => options.iter().position(|&option| option == key),
            serde_json::Value::Number(number) => number
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| n < options.len()),
            _ => None,
        }
        .ok_or_else(invalid)?;
        let confidence = answer
            .and_then(|a| a.get("confidence"))
            .and_then(serde_json::Value::as_f64);
        Ok((index, confidence))
    }
}

#[cfg(feature = "ollama")]
impl DecisionClient for OllamaDecisionClient {
    fn decide(&self, decision: &Decision<'_>) -> Result<Choice, DecisionError> {
        decision.validate()?;
        let body = ureq::post(&self.systemone_url())
            .send_json(self.build_request(decision))
            .map_err(|err| DecisionError::Http(err.to_string()))?
            .into_string()?;
        let (index, confidence) = Self::parse_response(&body, decision.options)?;
        Ok(Choice {
            index,
            confidence,
            model: self.model.clone(),
        })
    }
}

/// Builds a decision client from the environment, or `None` when no model is
/// opted in.
///
/// Opt-in is explicit: only a set, non-blank `AVER_DECISION_MODEL` enables the
/// client; the default model name alone never does. `AVER_DECISION_URL`
/// optionally overrides the Ollama base URL (default `http://localhost:11434`).
/// Without the `ollama` feature this always returns `None`.
#[cfg(feature = "ollama")]
pub fn client_from_env() -> Option<Box<dyn DecisionClient + Send + Sync>> {
    std::env::var("AVER_DECISION_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let config = DecisionConfig::from_env();
    Some(Box::new(OllamaDecisionClient::new(
        config.base_url,
        config.model,
    )))
}

/// See the `ollama`-feature variant; without the feature no client exists.
#[cfg(not(feature = "ollama"))]
pub fn client_from_env() -> Option<Box<dyn DecisionClient + Send + Sync>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision<'a>(options: &'a [&'a str]) -> Decision<'a> {
        Decision {
            state: "state",
            question: "question",
            options,
        }
    }

    #[test]
    fn validate_enforces_option_count_range() {
        let options: Vec<String> = (0..25).map(|i| format!("o{i}")).collect();
        let options: Vec<&str> = options.iter().map(String::as_str).collect();

        assert!(matches!(
            decision(&options[..0]).validate(),
            Err(DecisionError::TooFewOptions { count: 0 })
        ));
        assert!(matches!(
            decision(&options[..1]).validate(),
            Err(DecisionError::TooFewOptions { count: 1 })
        ));
        assert!(decision(&options[..2]).validate().is_ok());
        assert!(decision(&options[..24]).validate().is_ok());
        assert!(matches!(
            decision(&options[..25]).validate(),
            Err(DecisionError::TooManyOptions { count: 25 })
        ));
    }

    #[test]
    fn noop_client_is_unavailable() {
        let result = NoopDecisionClient.decide(&decision(&["a", "b"]));
        assert!(matches!(result, Err(DecisionError::Unavailable)));
    }

    #[test]
    fn decision_config_default() {
        let config = DecisionConfig::default();
        assert_eq!(config.base_url, "http://localhost:11434");
        assert_eq!(config.model, "tev1:4b");
    }

    #[test]
    fn mock_answers_in_order_and_records_decisions() {
        let client = MockDecisionClient::new(vec![Ok(1), Err(DecisionError::Unavailable), Ok(0)]);
        let first = Decision {
            state: "staged claim",
            question: "What happens to this staged claim?",
            options: &["promote", "reject", "hold"],
        };
        let second = decision(&["extract", "skip"]);

        assert_eq!(
            client.decide(&first).unwrap(),
            Choice {
                index: 1,
                confidence: None,
                model: MockDecisionClient::MODEL.to_string(),
            }
        );
        assert!(matches!(
            client.decide(&second),
            Err(DecisionError::Unavailable)
        ));
        assert_eq!(client.decide(&second).unwrap().index, 0);
        assert!(matches!(
            client.decide(&second),
            Err(DecisionError::Unavailable)
        ));

        let received = client.received();
        assert_eq!(received.len(), 4);
        assert_eq!(
            received[0],
            RecordedDecision {
                state: "staged claim".to_string(),
                question: "What happens to this staged claim?".to_string(),
                options: vec!["promote".into(), "reject".into(), "hold".into()],
            }
        );
        assert_eq!(received[1].options, vec!["extract", "skip"]);
    }

    #[test]
    fn mock_rejects_out_of_range_script_and_invalid_decisions() {
        let client = MockDecisionClient::with_indices(vec![2, 0]);
        assert!(matches!(
            client.decide(&decision(&["a", "b"])),
            Err(DecisionError::InvalidChoice { raw }) if raw == "2"
        ));
        assert!(matches!(
            client.decide(&decision(&["a"])),
            Err(DecisionError::TooFewOptions { count: 1 })
        ));
        assert_eq!(client.received().len(), 2);
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn ollama_client_trims_base_url_and_exposes_model() {
        let config = DecisionConfig::default();
        let client = OllamaDecisionClient::new("http://localhost:11434/", config.model.clone());
        assert_eq!(client.base_url, "http://localhost:11434");
        assert_eq!(client.model(), config.model);
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn systemone_url_and_request_shape() {
        let client = OllamaDecisionClient::new("http://localhost:11434/", "tev1:4b");
        assert_eq!(
            client.systemone_url(),
            "http://localhost:11434/v1/systemone"
        );
        let request = client.build_request(&Decision {
            state: "Claim: repo uses sqlite.",
            question: "Which scope does this candidate belong to?",
            options: &["global", "project", "session"],
        });
        assert_eq!(
            request,
            serde_json::json!({
                "model": "tev1:4b",
                "state": "Claim: repo uses sqlite.",
                "questions": {"decision": {
                    "type": "choice",
                    "instructions": "Which scope does this candidate belong to?",
                    "criteria": {"global": "global", "project": "project", "session": "session"},
                }},
            })
        );
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn parse_response_maps_key_or_index_with_confidence() {
        let options = ["global", "project", "session"];
        let by_key = r#"{"answers":{"decision":{"choice":"project","probabilities":{"global":0.1,"project":0.8,"session":0.1},"confidence":0.8}}}"#;
        assert_eq!(
            OllamaDecisionClient::parse_response(by_key, &options).unwrap(),
            (1, Some(0.8))
        );
        let by_index = r#"{"answers":[{"choice":2,"confidence":0.55}]}"#;
        assert_eq!(
            OllamaDecisionClient::parse_response(by_index, &options).unwrap(),
            (2, Some(0.55))
        );
        let no_confidence = r#"{"answers":{"decision":{"choice":"global"}}}"#;
        assert_eq!(
            OllamaDecisionClient::parse_response(no_confidence, &options).unwrap(),
            (0, None)
        );
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn parse_response_rejects_unknown_or_out_of_range_choice() {
        let options = ["global", "project"];
        for body in [
            r#"{"answers":{"decision":{"choice":"other"}}}"#,
            r#"{"answers":{"decision":{"choice":2}}}"#,
            r#"{"answers":{"decision":{"choice":-1}}}"#,
            r#"{"answers":{"decision":{"choice":null}}}"#,
            r#"{"answers":{"decision":{}}}"#,
            r#"{"answers":[]}"#,
            r#"{}"#,
        ] {
            assert!(
                matches!(
                    OllamaDecisionClient::parse_response(body, &options),
                    Err(DecisionError::InvalidChoice { .. })
                ),
                "{body}"
            );
        }
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn parse_response_rejects_malformed_json() {
        assert!(matches!(
            OllamaDecisionClient::parse_response("not json", &["a", "b"]),
            Err(DecisionError::Json(_))
        ));
    }

    #[test]
    fn validate_rejects_duplicate_and_empty_options() {
        assert!(matches!(
            decision(&["a", "b", "a"]).validate(),
            Err(DecisionError::DuplicateOption { option }) if option == "a"
        ));
        assert!(matches!(
            decision(&["a", ""]).validate(),
            Err(DecisionError::DuplicateOption { option }) if option.is_empty()
        ));
    }

    #[test]
    fn client_from_env_is_none_when_model_unset() {
        // No test in this crate sets AVER_DECISION_MODEL, so this cannot race.
        if std::env::var_os("AVER_DECISION_MODEL").is_none() {
            assert!(client_from_env().is_none());
        }
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn parse_response_accepts_live_tev1_response() {
        let body = r#"{"model":"tev1:4b","answers":{"decision":{"type":"choice","choice":"promote","probabilities":{"promote":0.8652696265547987,"reject":0.0638116585936579,"hold":0.07091871485154351},"confidence":0.7979044398321978}},"usage":{"input_tokens":146,"output_tokens":0}}"#;
        let (index, confidence) =
            OllamaDecisionClient::parse_response(body, &["promote", "reject", "hold"]).unwrap();
        assert_eq!(index, 0);
        assert!((confidence.unwrap() - 0.7979044398321978).abs() < 1e-12);
    }
}
