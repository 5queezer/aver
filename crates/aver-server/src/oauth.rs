use base64::Engine;
use sha2::{Digest, Sha256};

pub fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    pkce_s256_challenge(verifier) == challenge
}

pub fn authorization_server_metadata(base_url: &str) -> serde_json::Value {
    let base = base_url.trim_end_matches('/');
    serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    })
}
