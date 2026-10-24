use aver_core::{
    AgentKind, GraphPathMode, GraphPathQuery, HyperedgeInput, HyperedgeParticipantInput,
    Provenance, RelationshipKind, ScopeWalk, Store,
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
        ..GraphPathQuery::default()
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

fn scoped_query(from: &str, to: &str, scope: &str, walk: ScopeWalk) -> GraphPathQuery {
    GraphPathQuery {
        scope: scope.to_string(),
        walk,
        ..GraphPathQuery::new(from, to)
    }
}

fn api_prod_hyperedge() -> HyperedgeInput {
    HyperedgeInput {
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
    }
}

#[test]
fn default_query_is_unscoped_and_rejects_invalid_scope() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .add_claim_with_scope("A", "depends_on", "B", "s1", "proj/x")
        .unwrap();

    let query = GraphPathQuery::new("A", "B");
    assert_eq!(query.scope, "global");
    assert_eq!(query.walk, ScopeWalk::Any);
    assert!(store.graph_path(query).unwrap().is_some());

    let err = store
        .graph_path(scoped_query("A", "B", "bad scope!", ScopeWalk::Exact))
        .unwrap_err();
    assert!(matches!(err, aver_core::Error::InvalidScope { .. }));
}

#[test]
fn scoped_path_follows_ancestor_walk_and_hides_from_other_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project_claim = store
        .add_claim_with_scope("A", "depends_on", "B", "s1", "proj/x")
        .unwrap();
    let global_claim = store.add_claim("B", "depends_on", "C", "s2").unwrap();

    // proj/x + ancestors sees its own claim plus the inherited global one.
    let path = store
        .graph_path(scoped_query("A", "C", "proj/x", ScopeWalk::Ancestors))
        .unwrap()
        .unwrap();
    assert_eq!(path.entities(), vec!["A", "B", "C"]);
    assert_eq!(path.steps[0].claim_id, Some(project_claim));
    assert_eq!(path.steps[0].scope, "proj/x");
    assert_eq!(path.steps[1].claim_id, Some(global_claim));
    assert_eq!(path.steps[1].scope, "global");

    // A global-only view cannot reach the project-scoped edge.
    assert!(
        store
            .graph_path(scoped_query("A", "C", "global", ScopeWalk::Exact))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .graph_path(scoped_query("A", "B", "global", ScopeWalk::Ancestors))
            .unwrap()
            .is_none()
    );
    // A sibling project does not inherit proj/x.
    assert!(
        store
            .graph_path(scoped_query("A", "B", "proj/y", ScopeWalk::Ancestors))
            .unwrap()
            .is_none()
    );
    // Exact proj/x excludes the global edge but keeps the local one.
    assert!(
        store
            .graph_path(scoped_query("A", "C", "proj/x", ScopeWalk::Exact))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .graph_path(scoped_query("A", "B", "proj/x", ScopeWalk::Exact))
            .unwrap()
            .is_some()
    );
    // Descendants of "proj" include proj/x but not global.
    assert!(
        store
            .graph_path(scoped_query("A", "B", "proj", ScopeWalk::Descendants))
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .graph_path(scoped_query("B", "C", "proj", ScopeWalk::Descendants))
            .unwrap()
            .is_none()
    );
}

#[test]
fn hyperedges_are_treated_as_global_rows_for_scope_filtering() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_hyperedge(api_prod_hyperedge()).unwrap();

    // Any and Ancestors admit global rows from every scope.
    for walk in [ScopeWalk::Any, ScopeWalk::Ancestors] {
        let path = store
            .graph_path(scoped_query("api", "prod", "proj/x", walk))
            .unwrap()
            .unwrap();
        assert_eq!(path.steps[0].relationship_kind, RelationshipKind::Hyperedge);
        assert_eq!(path.steps[0].claim_id, None);
        assert_eq!(path.steps[0].scope, "global");
    }
    // Exact/Descendants admit hyperedges only from the global scope itself.
    for walk in [ScopeWalk::Exact, ScopeWalk::Descendants] {
        assert!(
            store
                .graph_path(scoped_query("api", "prod", "global", walk))
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .graph_path(scoped_query("api", "prod", "proj/x", walk))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn claim_steps_carry_claim_id_and_scope() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let id = store.add_claim("A", "depends_on", "B", "s1").unwrap();

    let path = store.graph_path(user_query("A", "B")).unwrap().unwrap();
    assert_eq!(path.steps[0].claim_id, Some(id));
    assert_eq!(path.steps[0].scope, "global");
}
