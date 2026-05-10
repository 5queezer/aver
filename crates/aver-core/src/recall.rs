use std::collections::HashSet;

use crate::Claim;

pub(crate) fn graph_score_for_query_claim(query: &str, claim: &Claim) -> f64 {
    let query_tokens: HashSet<String> = query_tokens_for_recall(query).into_iter().collect();
    if query_tokens.is_empty() {
        return 0.0;
    }
    let endpoint_tokens: HashSet<String> =
        tokenize_for_recall(&format!("{} {}", claim.subject, claim.object))
            .into_iter()
            .collect();
    if endpoint_tokens.is_empty() {
        return 0.0;
    }
    let overlap = query_tokens.intersection(&endpoint_tokens).count() as f64;
    overlap / query_tokens.len() as f64
}

pub(crate) fn query_tokens_for_recall(query: &str) -> Vec<String> {
    let mut tokens = tokenize_for_recall(query);
    if tokens.len() >= 3 {
        let acronym: String = tokens
            .iter()
            .filter_map(|token| token.chars().next())
            .collect();
        if acronym.len() >= 2 && !tokens.contains(&acronym) {
            tokens.push(acronym);
        }
    }
    tokens
}

pub(crate) fn tokenize_for_recall(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .flat_map(camel_case_parts)
        .map(|token| normalize_recall_token(&token))
        .collect()
}

pub(crate) fn camel_case_parts(token: &str) -> Vec<String> {
    if token
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        let mut parts = vec![token.to_string()];
        if let Some(split) = token.find(|ch: char| ch.is_ascii_digit())
            && split > 0
        {
            parts.push(token[..split].to_string());
            let digits = &token[split..];
            parts.push(digits.to_string());
            if digits.len() > 1 {
                parts.extend(digits.chars().map(|digit| digit.to_string()));
            }
        }
        return parts;
    }

    let mut parts = Vec::new();
    let mut start = 0;
    for (idx, ch) in token.char_indices().skip(1) {
        if ch.is_ascii_uppercase() {
            parts.push(token[start..idx].to_string());
            start = idx;
        }
    }
    parts.push(token[start..].to_string());
    let base_parts = parts.clone();
    for part in &base_parts {
        if let Some(split) = part.find(|ch: char| ch.is_ascii_digit())
            && split > 0
        {
            parts.push(part[..split].to_string());
            let digits = &part[split..];
            parts.push(digits.to_string());
            if digits.len() > 1 {
                parts.extend(digits.chars().map(|digit| digit.to_string()));
            }
        }
    }
    if base_parts.len() >= 2 {
        let acronym: String = base_parts
            .iter()
            .filter_map(|part| part.chars().next())
            .collect();
        parts.push(acronym);
    }
    parts
}

pub(crate) fn normalize_recall_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if lower == "children" {
        "child".to_string()
    } else if lower == "people" {
        "person".to_string()
    } else if lower.len() > 4 && lower.ends_with("ee") {
        lower.trim_end_matches("ee").to_string()
    } else if lower.len() > 4 && lower.ends_with("ies") {
        format!("{}y", lower.trim_end_matches("ies"))
    } else if lower.len() > 3 && lower.ends_with('s') {
        lower.trim_end_matches('s').to_string()
    } else {
        lower
    }
}

pub(crate) fn recall_token_score(query_tokens: &[String], claim: &Claim) -> usize {
    let claim_text = claim.text();
    let exact_text = claim_text.to_ascii_lowercase();
    let exact_tokens: HashSet<String> = exact_text
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let sub_tokens: HashSet<String> = tokenize_for_recall(&claim_text).into_iter().collect();
    query_tokens
        .iter()
        .map(|token| {
            if exact_tokens.contains(token) {
                2
            } else if sub_tokens.contains(token) {
                1
            } else {
                0
            }
        })
        .sum()
}
