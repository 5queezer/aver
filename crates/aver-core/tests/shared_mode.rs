//! T102 — v0.9 starts by declaring a shared-mode feature flag.

use aver_core::{GraphStorageAdapter, StorageMode};

#[test]
fn workspace_declares_shared_mode_feature() {
    let cargo_toml = include_str!("../../../Cargo.toml");

    assert!(cargo_toml.contains("shared_mode"));
}

#[test]
fn local_store_exposes_storage_adapter_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let store = aver_core::Store::open(dir.path()).unwrap();

    assert_eq!(store.mode(), StorageMode::Local);
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
        communities[0].id,
        "community:BillingTeam-PaymentGateway-StripeSDK"
    );
    assert_eq!(
        communities[0].members,
        vec![
            "BillingTeam".to_string(),
            "PaymentGateway".to_string(),
            "StripeSDK".to_string()
        ]
    );
    assert_eq!(communities[1].id, "community:Facts-Parser");
    assert_eq!(
        communities[1].members,
        vec!["Facts".to_string(), "Parser".to_string()]
    );
}

#[test]
fn agent_trust_score_uses_bounded_agreement_rate() {
    let dir = tempfile::tempdir().unwrap();
    let store = aver_core::Store::open(dir.path()).unwrap();
    store
        .add_claim_from_agent(
            "parser_agent",
            aver_core::AgentKind::DeterministicParser,
            "Parser",
            "emits",
            "Facts",
            "s1",
        )
        .unwrap();
    store
        .add_claim_from_agent(
            "parser_agent",
            aver_core::AgentKind::DeterministicParser,
            "Parser",
            "emits",
            "Triples",
            "s2",
        )
        .unwrap();
    store.consolidate().unwrap();

    assert_eq!(store.agent_trust_score("new_agent").unwrap(), 0.5);
    assert_eq!(store.agent_trust_score("parser_agent").unwrap(), 0.5);
}
