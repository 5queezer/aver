use crate::Error;
use crate::recall::query_tokens_for_recall;

pub(crate) fn validate_vector_chunk_text(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidVectorChunkText)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_embedding_model(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidEmbeddingModel)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_embedding_vector(value: &[f32]) -> Result<(), Error> {
    if value.is_empty() || value.iter().any(|component| !component.is_finite()) {
        Err(Error::InvalidEmbeddingVector)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_top_k(top_k: usize) -> Result<(), Error> {
    if top_k == 0 {
        Err(Error::InvalidTopK)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_claim_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidClaimField { field })
    } else {
        Ok(())
    }
}

pub(crate) fn validate_contradiction_reason(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidContradictionReason)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_candidate_status_filter(value: &str) -> Result<(), Error> {
    match value {
        "PENDING" | "PROMOTED" | "REJECTED" => Ok(()),
        _ => Err(Error::InvalidCandidateStatusFilter {
            status: value.to_string(),
        }),
    }
}

pub(crate) fn validate_recall_query(value: &str) -> Result<(), Error> {
    if query_tokens_for_recall(value).is_empty() {
        Err(Error::InvalidRecallQuery)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_rejection_reason(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidRejectionReason)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_event_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidEventField { field })
    } else {
        Ok(())
    }
}

pub(crate) fn validate_observation_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidObservationField { field })
    } else {
        Ok(())
    }
}

pub(crate) fn validate_agent_id(agent_id: &str) -> Result<(), Error> {
    if agent_id.is_empty()
        || !agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(Error::InvalidAgentId {
            value: agent_id.to_string(),
        });
    }
    Ok(())
}

/// ADR-0021 scope validation: non-blank, charset `[A-Za-z0-9_/-]`. Mirrors the
/// SQL trigger contract but rejects in Rust before we hit sqlite, giving a
/// typed error and avoiding a wasted privacy-filter pass.
pub(crate) fn validate_scope(scope: &str) -> Result<(), Error> {
    if scope.trim().is_empty() {
        return Err(Error::InvalidScope {
            value: scope.to_string(),
        });
    }
    let charset_ok = scope
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'/');
    if !charset_ok {
        return Err(Error::InvalidScope {
            value: scope.to_string(),
        });
    }
    Ok(())
}
