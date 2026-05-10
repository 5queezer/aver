use aver_core::{
    AgentKind, GraphPathMode, GraphPathQuery, HyperedgeInput, HyperedgeParticipantInput,
    Provenance, RelationshipKind, Store,
};

fn user_query(from: &str, to: &str) -> GraphPathQuery {
    GraphPathQuery {
        source: from.to_string(),
        target: to.to_string(),
        min_confidence: 0.0,
        allowed_provenance: None,
        max_hops: 8,
        predicates: None,
        mode: GraphPathMode::Directed,
    }
}

#[test]
fn directed_paths_preserve_claim_direction_unless_bidirectional() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_claim("A", "depends_on", "B", "s1").unwrap();

    assert!(store.graph_path(user_query("B", "A")).unwrap().is_none());

    let mut query = user_query("B", "A");
    query.mode = GraphPathMode::Bidirectional;
    let path = store.graph_path(query).unwrap().unwrap();
    assert_eq!(path.steps.len(), 1);
    assert_eq!(path.steps[0].source, "A");
    assert_eq!(path.steps[0].target, "B");
    assert_eq!(path.steps[0].predicate, "depends_on");
    assert_eq!(path.steps[0].relationship_kind, RelationshipKind::Claim);
}

#[test]
fn shortest_path_prefers_higher_confidence_when_hop_count_ties() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim_with_confidence("A", "depends_on", "Low", "s-low", 0.6)
        .unwrap();
    store
        .add_claim_with_confidence("Low", "depends_on", "D", "s-low", 0.6)
        .unwrap();
    store
        .add_claim_with_confidence("A", "depends_on", "High", "s-high", 0.9)
        .unwrap();
    store
        .add_claim_with_confidence("High", "depends_on", "D", "s-high", 0.8)
        .unwrap();

    let path = store.graph_path(user_query("A", "D")).unwrap().unwrap();

    assert_eq!(path.entities(), vec!["A", "High", "D"]);
    assert!((path.confidence - 0.72).abs() < 1e-9);
}

#[test]
fn multi_hop_path_confidence_decays_multiplicatively() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim_with_confidence("A", "depends_on", "B", "s1", 0.9)
        .unwrap();
    store
        .add_claim_with_confidence("B", "depends_on", "C", "s2", 0.8)
        .unwrap();
    store
        .add_claim_with_confidence("C", "depends_on", "D", "s3", 0.5)
        .unwrap();

    let path = store.graph_path(user_query("A", "D")).unwrap().unwrap();

    assert_eq!(path.entities(), vec!["A", "B", "C", "D"]);
    assert!((path.confidence - 0.36).abs() < 1e-9);
}

#[test]
fn confidence_floor_filters_claims_and_returns_no_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim_with_confidence("A", "depends_on", "B", "s1", 0.4)
        .unwrap();

    let mut query = user_query("A", "B");
    query.min_confidence = 0.5;

    assert!(store.graph_path(query).unwrap().is_none());
}

#[test]
fn provenance_filter_limits_path_edges() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim("A", "depends_on", "UserOnly", "s1")
        .unwrap();
    store
        .add_claim_from_agent(
            "parser_agent",
            AgentKind::DeterministicParser,
            "A",
            "depends_on",
            "ExtractedOnly",
            "s2",
        )
        .unwrap();

    let mut query = user_query("A", "ExtractedOnly");
    query.allowed_provenance = Some(vec![Provenance::UserAsserted]);
    assert!(store.graph_path(query.clone()).unwrap().is_none());

    query.allowed_provenance = Some(vec![Provenance::Extracted]);
    let path = store.graph_path(query).unwrap().unwrap();
    assert_eq!(path.steps[0].provenance, Provenance::Extracted);
}

#[test]
fn path_can_traverse_active_hyperedges_between_participants() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_hyperedge(HyperedgeInput {
            predicate: "deployed_with".to_string(),
            provenance: Provenance::UserAsserted,
            confidence: 0.88,
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

    let path = store
        .graph_path(user_query("api", "prod"))
        .unwrap()
        .unwrap();

    assert_eq!(path.steps.len(), 1);
    assert_eq!(path.steps[0].source, "api");
    assert_eq!(path.steps[0].target, "prod");
    assert_eq!(path.steps[0].predicate, "deployed_with");
    assert_eq!(path.steps[0].confidence, 0.88);
    assert_eq!(path.steps[0].provenance, Provenance::UserAsserted);
    assert_eq!(path.steps[0].relationship_kind, RelationshipKind::Hyperedge);
    assert_eq!(path.steps[0].source_refs, vec!["deploy-log".to_string()]);
}

#[test]
fn no_path_returns_none_when_max_hops_exhausted() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_claim("A", "depends_on", "B", "s1").unwrap();
    store.add_claim("B", "depends_on", "C", "s2").unwrap();

    let mut query = user_query("A", "C");
    query.max_hops = 1;

    assert!(store.graph_path(query).unwrap().is_none());
}

#[test]
fn predicate_filter_matches_ontology_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_claim("A", "calls", "B", "s1").unwrap();

    let mut query = user_query("A", "B");
    query.predicates = Some(vec!["depends_on".to_string()]);

    let path = store.graph_path(query).unwrap().unwrap();
    assert_eq!(path.steps[0].predicate, "calls");
}
