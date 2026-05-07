use memory_server::oauth::{pkce_s256_challenge, verify_pkce_s256};

#[test]
fn pkce_s256_verification_matches_rfc_example() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    assert_eq!(pkce_s256_challenge(verifier), challenge);
    assert!(verify_pkce_s256(verifier, challenge));
    assert!(!verify_pkce_s256("wrong", challenge));
}
