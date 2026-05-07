use memory_server::oauth::authorization_server_metadata;

#[test]
fn oauth_metadata_advertises_pkce_authorization_code_flow() {
    let metadata = authorization_server_metadata("https://aml.example.com");

    assert_eq!(metadata["issuer"], "https://aml.example.com");
    assert_eq!(
        metadata["authorization_endpoint"],
        "https://aml.example.com/oauth/authorize"
    );
    assert_eq!(
        metadata["token_endpoint"],
        "https://aml.example.com/oauth/token"
    );
    assert_eq!(
        metadata["registration_endpoint"],
        "https://aml.example.com/oauth/register"
    );
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
    assert_eq!(
        metadata["grant_types_supported"],
        serde_json::json!(["authorization_code", "refresh_token"])
    );
}
