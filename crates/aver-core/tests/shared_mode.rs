//! T102 — v0.9 starts by declaring a shared-mode feature flag.

#[test]
fn workspace_declares_shared_mode_feature() {
    let cargo_toml = include_str!("../../../Cargo.toml");

    assert!(cargo_toml.contains("shared_mode"));
}

#[test]
fn detect_communities_groups_connected_claim_entities() {
    let dir = tempfile::tempdir().unwrap();
    let store = aver_core::Store::open(dir.path()).unwrap();
    store
        .add_claim("PaymentGateway", "depends_on", "StripeSDK", "s1")
        .unwrap();
    store
        .add_claim("StripeSDK", "owned_by", "BillingTeam", "s2")
        .unwrap();
    store.add_claim("Parser", "emits", "Facts", "s3").unwrap();

    let communities = store.detect_communities().unwrap();

    assert_eq!(communities.len(), 2);
    assert_eq!(
        communities[0].members,
        vec![
            "PaymentGateway".to_string(),
            "StripeSDK".to_string(),
            "BillingTeam".to_string()
        ]
    );
    assert_eq!(
        communities[1].members,
        vec!["Parser".to_string(), "Facts".to_string()]
    );
}
