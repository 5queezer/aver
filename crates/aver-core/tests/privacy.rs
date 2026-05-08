//! T72 — v0.4 starts with a pre-write privacy detector for obvious secrets.

use aver_core::{Error, PrivacyRejection, Store, privacy_filter, privacy_filter_path};

fn synthetic_token(parts: &[&str]) -> String {
    parts.concat()
}

#[test]
fn privacy_filter_rejects_aws_access_key() {
    let token = synthetic_token(&["AK", "IA", "ABCDEFGHIJKLMNOP"]);
    let result = privacy_filter(&format!("deploy key {token} should never persist"));

    assert_eq!(result, Err(PrivacyRejection::AwsAccessKey));
}

#[test]
fn privacy_filter_rejects_github_pat() {
    let token = synthetic_token(&["gh", "p_", "abcdefghijklmnopqrstuvwxyz1234567890abcd"]);
    let result = privacy_filter(&format!("token {token}"));

    assert_eq!(result, Err(PrivacyRejection::GitHubPat));
}

#[test]
fn privacy_filter_rejects_fine_grained_github_pat() {
    let token = synthetic_token(&["github", "_pat_", "11ABCDEFG0abcdefghijklmnopqrstuvwxyz"]);
    let result = privacy_filter(&format!("token {token}"));

    assert_eq!(result, Err(PrivacyRejection::GitHubFineGrainedPat));
}

#[test]
fn privacy_filter_rejects_jwt() {
    let token = synthetic_token(&[
        "eyJhbGciOiJIUzI1NiJ9",
        ".",
        "eyJzdWIiOiIxMjM0NTY3ODkw",
        ".signature123",
    ]);
    let result = privacy_filter(&format!("bearer {token}"));

    assert_eq!(result, Err(PrivacyRejection::Jwt));
}

#[test]
fn privacy_filter_rejects_openai_key() {
    let token = synthetic_token(&["sk", "-", "abcdefghijklmnopqrstuvwxyz1234567890"]);
    let result = privacy_filter(&format!("OPENAI_API_KEY={token}"));

    assert_eq!(result, Err(PrivacyRejection::OpenAiKey));
}

#[test]
fn privacy_filter_rejects_anthropic_key() {
    let token = synthetic_token(&["sk", "-ant-", "abcdefghijklmnopqrstuvwxyz1234567890"]);
    let result = privacy_filter(&format!("ANTHROPIC_API_KEY={token}"));

    assert_eq!(result, Err(PrivacyRejection::AnthropicKey));
}

#[test]
fn privacy_filter_rejects_stripe_live_key() {
    let token = synthetic_token(&["sk", "_live_", "abcdefghijklmnopqrstuvwxyz123456"]);
    let result = privacy_filter(&format!("STRIPE_SECRET={token}"));

    assert_eq!(result, Err(PrivacyRejection::StripeLiveKey));
}

#[test]
fn privacy_filter_rejects_private_key_header() {
    let marker = synthetic_token(&["-----BEGIN ", "PRIVATE KEY-----"]);
    let result = privacy_filter(&format!("{marker}\nabc\n-----END PRIVATE KEY-----"));

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
fn privacy_filter_path_rejects_aws_credentials_file() {
    let result = privacy_filter_path("/home/alice/.aws/credentials");

    assert_eq!(result, Err(PrivacyRejection::AwsCredentialsPath));
}

#[test]
fn privacy_filter_path_rejects_config_dir() {
    let result = privacy_filter_path("/home/alice/.config/gh/hosts.yml");

    assert_eq!(result, Err(PrivacyRejection::ConfigPath));
}

#[test]
fn privacy_filter_path_rejects_pem_file() {
    let result = privacy_filter_path("/project/certs/prod.pem");

    assert_eq!(result, Err(PrivacyRejection::KeyPath));
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

    let token = synthetic_token(&["AK", "IA", "ABCDEFGHIJKLMNOP"]);
    let result = store.add_claim("deployment", "uses_key", &token, "test_session");

    assert!(matches!(
        result,
        Err(Error::Privacy(PrivacyRejection::AwsAccessKey))
    ));
    assert!(!dir.path().join("log.jsonl").exists());
}

#[test]
fn privacy_filter_path_rejects_relative_env_file() {
    assert_eq!(
        privacy_filter_path(".env").unwrap_err(),
        PrivacyRejection::EnvPath
    );
}

#[test]
fn privacy_filter_path_rejects_relative_ssh_dir() {
    assert_eq!(
        privacy_filter_path(".ssh/id_rsa").unwrap_err(),
        PrivacyRejection::SshPath
    );
}

#[test]
fn privacy_filter_path_rejects_relative_aws_credentials_file() {
    assert_eq!(
        privacy_filter_path(".aws/credentials").unwrap_err(),
        PrivacyRejection::AwsCredentialsPath
    );
}

#[test]
fn privacy_filter_path_rejects_relative_config_dir() {
    assert_eq!(
        privacy_filter_path(".config/aver/secrets.toml").unwrap_err(),
        PrivacyRejection::ConfigPath
    );
}

#[test]
fn privacy_filter_path_rejects_relative_secrets_dir() {
    assert_eq!(
        privacy_filter_path(".secrets.d/token").unwrap_err(),
        PrivacyRejection::SecretsPath
    );
}

#[test]
fn privacy_filter_path_rejects_envrc_file() {
    assert_eq!(
        privacy_filter_path(".envrc").unwrap_err(),
        PrivacyRejection::EnvPath
    );
}

#[test]
fn privacy_filter_rejects_openssh_private_key_header() {
    let content = synthetic_token(&["-----BEGIN ", "OPENSSH PRIVATE KEY-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_rsa_private_key_header() {
    let content = synthetic_token(&["-----BEGIN ", "RSA PRIVATE KEY-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_ec_private_key_header() {
    let content = synthetic_token(&["-----BEGIN ", "EC PRIVATE KEY-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_dsa_private_key_header() {
    let content = synthetic_token(&["-----BEGIN ", "DSA PRIVATE KEY-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_encrypted_private_key_header() {
    let content = synthetic_token(&["-----BEGIN ", "ENCRYPTED PRIVATE KEY-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_pgp_private_key_block_header() {
    let content = synthetic_token(&["-----BEGIN ", "PGP PRIVATE KEY BLOCK-----"]);

    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_path_rejects_pkcs12_file() {
    assert_eq!(
        privacy_filter_path("certs/client-identity.p12").unwrap_err(),
        PrivacyRejection::KeyPath
    );
}

#[test]
fn privacy_filter_path_rejects_pfx_file() {
    assert_eq!(
        privacy_filter_path("certs/client-identity.pfx").unwrap_err(),
        PrivacyRejection::KeyPath
    );
}

#[test]
fn privacy_filter_path_rejects_java_keystore_file() {
    assert_eq!(
        privacy_filter_path("certs/production-keystore.jks").unwrap_err(),
        PrivacyRejection::KeyPath
    );
}

#[test]
fn privacy_filter_path_rejects_keystore_file() {
    assert_eq!(
        privacy_filter_path("android/release.keystore").unwrap_err(),
        PrivacyRejection::KeyPath
    );
}

#[test]
fn privacy_filter_path_rejects_password_database_file() {
    assert_eq!(
        privacy_filter_path("vault/team-passwords.kdbx").unwrap_err(),
        PrivacyRejection::SecretsPath
    );
}

#[test]
fn privacy_filter_path_rejects_legacy_password_database_file() {
    assert_eq!(
        privacy_filter_path("vault/team-passwords.kdb").unwrap_err(),
        PrivacyRejection::SecretsPath
    );
}

#[test]
fn privacy_filter_rejects_putty_private_key_header() {
    let content = synthetic_token(&["PuTTY-User-Key-", "File-3: ssh-rsa"]);
    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_rejects_ssh2_private_key_header() {
    let content = synthetic_token(&["---- BEGIN SSH2 ENCRYPTED ", "PRIVATE KEY ----"]);
    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_path_rejects_putty_key_file() {
    assert_eq!(
        privacy_filter_path("keys/deploy.ppk").unwrap_err(),
        PrivacyRejection::KeyPath
    );
}

#[test]
fn privacy_filter_rejects_age_secret_key() {
    let content = synthetic_token(&[
        "AGE-SECRET-KEY-",
        "1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ",
    ]);
    assert_eq!(
        privacy_filter(&content).unwrap_err(),
        PrivacyRejection::PrivateKey
    );
}

#[test]
fn privacy_filter_path_rejects_age_identity_dir() {
    assert_eq!(
        privacy_filter_path(".age/keys.txt").unwrap_err(),
        PrivacyRejection::SecretsPath
    );
}
