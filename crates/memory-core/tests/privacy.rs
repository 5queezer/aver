//! T72 — v0.4 starts with a pre-write privacy detector for obvious secrets.

use memory_core::{PrivacyRejection, privacy_filter};

#[test]
fn privacy_filter_rejects_aws_access_key() {
    let result = privacy_filter("deploy key AWS_ACCESS_KEY_TEST_TOKEN should never persist");

    assert_eq!(result, Err(PrivacyRejection::AwsAccessKey));
}
