use aver_server::oauth::authorization_server_metadata;

#[test]
fn oauth_metadata_advertises_pkce_authorization_code_flow() {
    let metadata = authorization_server_metadata("https://aver.example.com");

    assert_eq!(metadata["issuer"], "https://aver.example.com");
    assert_eq!(
        metadata["authorization_endpoint"],
        "https://aver.example.com/oauth/authorize"
    );
    assert_eq!(
        metadata["token_endpoint"],
        "https://aver.example.com/oauth/token"
    );
    assert_eq!(
        metadata["registration_endpoint"],
        "https://aver.example.com/oauth/register"
    );
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
    assert_eq!(
        metadata["grant_types_supported"],
        serde_json::json!(["authorization_code", "refresh_token"])
    );
    assert_eq!(
        metadata["token_endpoint_auth_methods_supported"],
        serde_json::json!(["none"])
    );
    assert_eq!(
        metadata["scopes_supported"],
        serde_json::json!([
            "claims:read",
            "claims:write",
            "events:write",
            "candidates:manage",
            "observations:read",
            "observations:write",
        ]),
    );
}
