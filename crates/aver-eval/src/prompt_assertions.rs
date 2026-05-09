use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamPromptVersion {
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    BeamAnswer(BeamPromptVersion),
    BeamJudge(BeamPromptVersion),
}

pub fn contract_for(kind: PromptKind) -> PromptContractSpec {
    match kind {
        PromptKind::BeamAnswer(BeamPromptVersion::V1) => PromptContractSpec::new("beam_answer_v1")
            .require_text("using only the retrieved memory context")
            .require_text("Answer the question directly")
            .require_text("Do not respond with generic meta-commentary")
            .require_text("If retrieved memories conflict")
            .require_text("If the context is insufficient, say you do not know")
            .forbid_text("ground-truth-only-answer")
            .require_section("QUESTION:")
            .require_section("RETRIEVED MEMORY CONTEXT:")
            .require_section("ANSWER:")
            .max_chars(20_000),
        PromptKind::BeamJudge(BeamPromptVersion::V1) => PromptContractSpec::new("beam_judge_v1")
            .require_text("expert evaluator")
            .require_text("RUBRIC CRITERION")
            .require_text("Return only JSON")
            .require_text(r#"{"score": 1.0, "reason": "..."}"#)
            .forbid_text("{{")
            .forbid_text("}}")
            .require_section("## EVALUATION INPUTS")
            .require_section("## SCORING SCALE:")
            .max_chars(16_000),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContractSpec {
    pub id: String,
    pub required_text: Vec<String>,
    pub forbidden_text: Vec<String>,
    pub required_sections: Vec<String>,
    pub max_chars: Option<usize>,
    pub reject_unresolved_placeholders: bool,
}

impl PromptContractSpec {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            required_text: Vec::new(),
            forbidden_text: Vec::new(),
            required_sections: Vec::new(),
            max_chars: None,
            reject_unresolved_placeholders: true,
        }
    }

    pub fn require_text(mut self, text: impl Into<String>) -> Self {
        self.required_text.push(text.into());
        self
    }

    pub fn forbid_text(mut self, text: impl Into<String>) -> Self {
        self.forbidden_text.push(text.into());
        self
    }

    pub fn require_section(mut self, section: impl Into<String>) -> Self {
        self.required_sections.push(section.into());
        self
    }

    pub fn max_chars(mut self, max_chars: usize) -> Self {
        self.max_chars = Some(max_chars);
        self
    }

    pub fn reject_unresolved_placeholders(mut self, reject: bool) -> Self {
        self.reject_unresolved_placeholders = reject;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContractError {
    pub contract_id: String,
    pub failures: Vec<String>,
}

impl fmt::Display for PromptContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "prompt contract {} failed: {}",
            self.contract_id,
            self.failures.join("; ")
        )
    }
}

impl std::error::Error for PromptContractError {}

pub fn assert_prompt_contract(
    prompt: &str,
    contract: &PromptContractSpec,
) -> Result<(), PromptContractError> {
    let mut failures = Vec::new();

    for required in &contract.required_text {
        if !prompt.contains(required) {
            failures.push(format!("missing required text: {required}"));
        }
    }

    for forbidden in &contract.forbidden_text {
        if prompt.contains(forbidden) {
            failures.push(format!("contains forbidden text: {forbidden}"));
        }
    }

    for section in &contract.required_sections {
        if !prompt.contains(section) {
            failures.push(format!("missing required section: {section}"));
        }
    }

    if contract.reject_unresolved_placeholders
        && (prompt.contains("{{") || prompt.contains("}}") || prompt.contains("<TODO>"))
    {
        failures.push("contains unresolved template placeholder".to_string());
    }

    if let Some(max_chars) = contract.max_chars {
        let actual_chars = prompt.chars().count();
        if actual_chars > max_chars {
            failures.push(format!(
                "prompt length {actual_chars} exceeds max {max_chars} chars"
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(PromptContractError {
            contract_id: contract.id.clone(),
            failures,
        })
    }
}
