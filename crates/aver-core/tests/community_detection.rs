use aver_core::ClaimWrite;
use aver_core::{HyperedgeInput, HyperedgeParticipantInput, Provenance, ScopeWalk, Store};

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
        .add_claim(ClaimWrite::new("A1", "depends_on", "A2", "a12").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("A2", "depends_on", "A3", "a23").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("A1", "depends_on", "A3", "a13").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("B1", "depends_on", "B2", "b12").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("B2", "depends_on", "B3", "b23").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("B1", "depends_on", "B3", "b13").confidence(0.95))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("A3", "depends_on", "B1", "weak-bridge").confidence(0.1))
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
        .add_claim(ClaimWrite::new("Hub", "depends_on", "Weak", "low").confidence(0.2))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("Hub", "depends_on", "Strong", "high").confidence(0.9))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("Strong", "depends_on", "Partner", "high").confidence(0.9))
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

#[test]
fn scoped_detection_excludes_out_of_scope_entities_and_rescoring_follows() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim(
            ClaimWrite::new("A", "depends_on", "B", "ab")
                .confidence(0.95)
                .scope("proj/x"),
        )
        .unwrap();
    store
        .add_claim(ClaimWrite::new("B", "depends_on", "C", "bc").confidence(0.7))
        .unwrap();
    store
        .add_claim(
            ClaimWrite::new("C", "depends_on", "D", "cd")
                .confidence(0.9)
                .scope("proj/y"),
        )
        .unwrap();

    let unscoped = store.detect_communities().unwrap();
    assert_eq!(unscoped.len(), 1);
    assert_eq!(unscoped[0].members, vec!["A", "B", "C", "D"]);
    assert!((unscoped[0].score - (0.95 + 0.7 + 0.9) / 3.0).abs() < 1e-9);
    assert_eq!(
        store
            .detect_communities_with_scope("global", ScopeWalk::Any)
            .unwrap(),
        unscoped
    );

    // proj/x + ancestors: A-B (local) and B-C (global); D is out of scope.
    let proj_x = store
        .detect_communities_with_scope("proj/x", ScopeWalk::Ancestors)
        .unwrap();
    assert_eq!(proj_x.len(), 1);
    assert_eq!(proj_x[0].members, vec!["A", "B", "C"]);
    assert!((proj_x[0].score - (0.95 + 0.7) / 2.0).abs() < 1e-9);
    assert!(proj_x[0].bridge_nodes.is_empty());

    // Exact proj/x: only the local edge remains, score is its own weight.
    let exact = store
        .detect_communities_with_scope("proj/x", ScopeWalk::Exact)
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].members, vec!["A", "B"]);
    assert!((exact[0].score - 0.95).abs() < 1e-9);

    // Exact global: only B-C survives; A and D vanish and the score drops to
    // the single remaining edge weight.
    let global = store
        .detect_communities_with_scope("global", ScopeWalk::Exact)
        .unwrap();
    assert_eq!(community_members(&global), vec![vec!["B", "C"]]);
    assert!((global[0].score - 0.7).abs() < 1e-9);
    assert!(global[0].bridge_nodes.is_empty());
}

#[test]
fn scoped_detection_admits_hyperedges_only_where_global_rows_are_visible() {
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
            ],
        })
        .unwrap();

    let ancestors = store
        .detect_communities_with_scope("proj/x", ScopeWalk::Ancestors)
        .unwrap();
    assert_eq!(community_members(&ancestors), vec![vec!["api", "prod"]]);

    let exact_global = store
        .detect_communities_with_scope("global", ScopeWalk::Exact)
        .unwrap();
    assert_eq!(community_members(&exact_global), vec![vec!["api", "prod"]]);

    assert!(
        store
            .detect_communities_with_scope("proj/x", ScopeWalk::Exact)
            .unwrap()
            .is_empty()
    );
}
