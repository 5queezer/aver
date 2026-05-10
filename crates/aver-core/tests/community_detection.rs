use aver_core::{HyperedgeInput, HyperedgeParticipantInput, Provenance, Store};

fn community_members(communities: &[aver_core::Community]) -> Vec<Vec<String>> {
    communities
        .iter()
        .map(|community| community.members.clone())
        .collect()
}

#[test]
fn weighted_detection_splits_dense_groups_joined_only_by_weak_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .add_claim_with_confidence("A1", "depends_on", "A2", "a12", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("A2", "depends_on", "A3", "a23", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("A1", "depends_on", "A3", "a13", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("B1", "depends_on", "B2", "b12", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("B2", "depends_on", "B3", "b23", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("B1", "depends_on", "B3", "b13", 0.95)
        .unwrap();
    store
        .add_claim_with_confidence("A3", "depends_on", "B1", "weak-bridge", 0.1)
        .unwrap();

    let communities = store.detect_communities().unwrap();

    assert_eq!(
        community_members(&communities),
        vec![
            vec!["A1".to_string(), "A2".to_string(), "A3".to_string()],
            vec!["B1".to_string(), "B2".to_string(), "B3".to_string()],
        ]
    );
    assert!(communities[0].score > 0.9);
    assert_eq!(communities[0].bridge_nodes, vec!["A3".to_string()]);
    assert_eq!(communities[1].bridge_nodes, vec!["B1".to_string()]);
}

#[test]
fn higher_confidence_edges_dominate_low_confidence_alternatives() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .add_claim_with_confidence("Hub", "depends_on", "Weak", "low", 0.2)
        .unwrap();
    store
        .add_claim_with_confidence("Hub", "depends_on", "Strong", "high", 0.9)
        .unwrap();
    store
        .add_claim_with_confidence("Strong", "depends_on", "Partner", "high", 0.9)
        .unwrap();

    let communities = store.detect_communities().unwrap();

    assert!(
        communities
            .iter()
            .any(|community| community.members == vec!["Hub", "Partner", "Strong"])
    );
    assert!(
        communities
            .iter()
            .any(|community| community.members == vec!["Weak"])
    );
}

#[test]
fn hyperedges_contribute_weighted_relationships_to_communities() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .add_hyperedge(HyperedgeInput {
            predicate: "deployed_with".to_string(),
            provenance: Provenance::UserAsserted,
            confidence: 0.9,
            source_refs: vec!["deploy-log".to_string()],
            participants: vec![
                HyperedgeParticipantInput {
                    role: "service".to_string(),
                    entity: "api".to_string(),
                },
                HyperedgeParticipantInput {
                    role: "environment".to_string(),
                    entity: "prod".to_string(),
                },
                HyperedgeParticipantInput {
                    role: "region".to_string(),
                    entity: "eu".to_string(),
                },
            ],
        })
        .unwrap();

    let communities = store.detect_communities().unwrap();

    assert_eq!(communities.len(), 1);
    assert_eq!(
        communities[0].members,
        vec!["api".to_string(), "eu".to_string(), "prod".to_string()]
    );
    assert!(communities[0].score >= 0.9);
}
