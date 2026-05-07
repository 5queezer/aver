//! T72 — v0.4 starts with a pre-write privacy detector for obvious secrets.

use memory_core::{Error, PrivacyRejection, Store, privacy_filter};

#[test]
fn privacy_filter_rejects_aws_access_key() {
    let result = privacy_filter("deploy key AWS_ACCESS_KEY_TEST_TOKEN should never persist");

    assert_eq!(result, Err(PrivacyRejection::AwsAccessKey));
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
