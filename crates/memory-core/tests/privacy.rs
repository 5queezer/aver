//! T72 — v0.4 starts with a pre-write privacy detector for obvious secrets.

use memory_core::{Error, PrivacyRejection, Store, privacy_filter, privacy_filter_path};

#[test]
fn privacy_filter_rejects_aws_access_key() {
    let result = privacy_filter("deploy key AWS_ACCESS_KEY_TEST_TOKEN should never persist");

    assert_eq!(result, Err(PrivacyRejection::AwsAccessKey));
}

#[test]
fn privacy_filter_rejects_github_pat() {
    let result = privacy_filter("token GITHUB_PAT_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::GitHubPat));
}

#[test]
fn privacy_filter_rejects_fine_grained_github_pat() {
    let result = privacy_filter("token GITHUB_FINE_GRAINED_PAT_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::GitHubFineGrainedPat));
}

#[test]
fn privacy_filter_rejects_jwt() {
    let result =
        privacy_filter("bearer JWT_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::Jwt));
}

#[test]
fn privacy_filter_rejects_openai_key() {
    let result = privacy_filter("OPENAI_API_KEY=OPENAI_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::OpenAiKey));
}

#[test]
fn privacy_filter_rejects_anthropic_key() {
    let result = privacy_filter("ANTHROPIC_API_KEY=ANTHROPIC_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::AnthropicKey));
}

#[test]
fn privacy_filter_rejects_stripe_live_key() {
    let result = privacy_filter("STRIPE_SECRET=STRIPE_TEST_TOKEN");

    assert_eq!(result, Err(PrivacyRejection::StripeLiveKey));
}

#[test]
fn privacy_filter_rejects_private_key_header() {
    let result = privacy_filter("PRIVATE_KEY_HEADER_TEST_TOKEN\nabc\n-----END PRIVATE KEY-----");

    assert_eq!(result, Err(PrivacyRejection::PrivateKey));
}

#[test]
fn privacy_filter_rejects_high_entropy_token() {
    let result = privacy_filter("secret q7Zp9Lm2Kx8Vn4Rb6Ty0Wc3Ae5Gu");

    assert_eq!(result, Err(PrivacyRejection::HighEntropy));
}

#[test]
fn privacy_filter_path_rejects_secrets_dir() {
    let result = privacy_filter_path("/home/alice/.secrets.d/openai");

    assert_eq!(result, Err(PrivacyRejection::SecretsPath));
}

#[test]
fn privacy_filter_path_rejects_env_file() {
    let result = privacy_filter_path("/project/.env.local");

    assert_eq!(result, Err(PrivacyRejection::EnvPath));
}

#[test]
fn privacy_filter_path_rejects_ssh_dir() {
    let result = privacy_filter_path("/home/alice/.ssh/id_ed25519");

    assert_eq!(result, Err(PrivacyRejection::SshPath));
}

#[test]
fn privacy_filter_rejects_memory_ignore_marker() {
    let result = privacy_filter("normal note\n# memory:ignore\nsecret context");

    assert_eq!(result, Err(PrivacyRejection::MemoryIgnore));
}

#[test]
fn privacy_filter_rejects_top_of_file_html_memory_ignore_marker() {
    let result = privacy_filter("<!-- memory:ignore -->\nproject notes");

    assert_eq!(result, Err(PrivacyRejection::MemoryIgnore));
}

#[test]
fn add_claim_rejects_secret_before_episodic_log_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let result = store.add_claim(
        "deployment",
        "uses_key",
        "AWS_ACCESS_KEY_TEST_TOKEN",
        "test_session",
    );

    assert!(matches!(
        result,
        Err(Error::Privacy(PrivacyRejection::AwsAccessKey))
    ));
    assert!(!dir.path().join("log.jsonl").exists());
}
