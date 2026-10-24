//! Roadmap Phase 5: `communities` and `path` over the core graph.

use aver_server::tools::{AddTripleParams, AverTools, CommunitiesParams, PathParams};

/// proj/a: A -> B -> C (B -> C at 0.6). proj/b: C -> D, X -> Y.
/// global: P -> Q at 0.3, too weak to group P and Q.
fn seeded(dir: &std::path::Path) -> AverTools {
    let tools = AverTools::open(dir).unwrap();
    for (subject, object, confidence, scope) in [
        ("A", "B", 0.95, "proj/a"),
        ("B", "C", 0.6, "proj/a"),
        ("C", "D", 0.95, "proj/b"),
        ("X", "Y", 0.95, "proj/b"),
        ("P", "Q", 0.3, "global"),
    ] {
        tools
            .add_triple(AddTripleParams {
                subject: subject.to_string(),
                predicate: "depends_on".to_string(),
                object: object.to_string(),
                confidence: Some(confidence),
                source: "graph-tools-test".to_string(),
                scope: Some(scope.to_string()),
            })
            .unwrap();
    }
    tools
}

fn communities(
    tools: &AverTools,
    scope: Option<&str>,
    walk: Option<&str>,
    min_members: Option<usize>,
    top_k: Option<usize>,
) -> Vec<Vec<String>> {
    tools
        .communities(CommunitiesParams {
            scope: scope.map(str::to_string),
            scope_walk: walk.map(str::to_string),
            min_members,
            top_k,
        })
        .unwrap()
        .communities
        .into_iter()
        .map(|community| {
            assert_eq!(community.size, community.members.len());
            community.members
        })
        .collect()
}

fn path_params(from: &str, to: &str) -> PathParams {
    PathParams {
        from: from.to_string(),
        to: to.to_string(),
        max_hops: None,
        predicates: None,
        min_confidence: None,
        scope: None,
        scope_walk: None,
    }
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn communities_groups_entities_and_respects_min_members_and_top_k() {
    let dir = tempfile::tempdir().unwrap();
    let tools = seeded(dir.path());

    let view = tools
        .communities(CommunitiesParams {
            scope: None,
            scope_walk: None,
            min_members: None,
            top_k: None,
        })
        .unwrap();
    assert_eq!(view.scope_walk, "any");
    let groups: Vec<Vec<String>> = view.communities.iter().map(|c| c.members.clone()).collect();
    assert_eq!(
        groups,
        vec![names(&["X", "Y"]), names(&["A", "B", "C", "D"])],
        "ranked by score: X-Y is 0.95, A..D averages in the 0.6 edge"
    );
    assert!(view.communities[1].score < view.communities[0].score);

    // P and Q are singletons (0.3 is below the strong-edge threshold) and
    // bridge to each other.
    let all = communities(&tools, None, None, Some(1), Some(100));
    assert_eq!(all.len(), 4);
    assert!(all.contains(&names(&["P"])) && all.contains(&names(&["Q"])));
    let singletons = tools
        .communities(CommunitiesParams {
            scope: None,
            scope_walk: None,
            min_members: Some(1),
            top_k: Some(100),
        })
        .unwrap();
    let p = singletons
        .communities
        .iter()
        .find(|c| c.members == names(&["P"]))
        .unwrap();
    assert_eq!(p.bridges, names(&["P"]));

    assert_eq!(
        communities(&tools, None, None, Some(3), None),
        vec![names(&["A", "B", "C", "D"])]
    );
    assert_eq!(
        communities(&tools, None, None, None, Some(1)),
        vec![names(&["X", "Y"])]
    );
    assert!(
        tools
            .communities(CommunitiesParams {
                scope: None,
                scope_walk: None,
                min_members: None,
                top_k: Some(101),
            })
            .is_err()
    );
}

#[test]
fn communities_filters_members_by_scope() {
    let dir = tempfile::tempdir().unwrap();
    let tools = seeded(dir.path());

    assert_eq!(
        communities(&tools, Some("proj/a"), Some("exact"), None, None),
        vec![names(&["A", "B", "C"])]
    );
    // Core groups and scores over the scoped claims only: C-D is no longer
    // diluted by proj/a's 0.6 edge, so it ties X-Y at 0.95 and ranks by id.
    let view = tools
        .communities(CommunitiesParams {
            scope: Some("proj/b".to_string()),
            scope_walk: Some("exact".to_string()),
            min_members: None,
            top_k: None,
        })
        .unwrap();
    let groups: Vec<Vec<String>> = view.communities.iter().map(|c| c.members.clone()).collect();
    assert_eq!(groups, vec![names(&["C", "D"]), names(&["X", "Y"])]);
    assert_eq!(view.communities[0].score, view.communities[1].score);
    // Ancestors of proj/a include global, but P and Q stay singletons.
    assert_eq!(
        communities(&tools, Some("proj/a"), None, Some(1), None).len(),
        3
    );
}

#[test]
fn path_finds_two_hop_path_with_claim_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let tools = seeded(dir.path());

    let view = tools.path(path_params("A", "C")).unwrap();
    assert!(view.found);
    assert_eq!(view.hops, 2);
    assert_eq!(view.path.len(), 2);
    assert_eq!(
        (
            view.path[0].subject.as_str(),
            view.path[0].object.as_str(),
            view.path[1].subject.as_str(),
            view.path[1].object.as_str(),
        ),
        ("A", "B", "B", "C")
    );
    assert_eq!(view.path[0].predicate, "depends_on");
    assert_eq!(view.path[0].provenance, "USER_ASSERTED");
    assert_eq!(view.path[1].confidence, 0.6);
    assert_eq!(view.path[0].scope, "proj/a");
    assert!(view.path[0].claim_id.is_some_and(|id| id > 0));
    assert!(view.path[1].claim_id.is_some() && view.path[0].claim_id != view.path[1].claim_id);
    assert!((view.confidence - 0.95 * 0.6).abs() < 1e-9);

    // Directed: no path back from C to A.
    assert!(!tools.path(path_params("C", "A")).unwrap().found);
}

#[test]
fn path_respects_max_hops_predicates_and_min_confidence() {
    let dir = tempfile::tempdir().unwrap();
    let tools = seeded(dir.path());

    let mut params = path_params("A", "C");
    params.max_hops = Some(1);
    let view = tools.path(params).unwrap();
    assert!(!view.found);
    assert_eq!(view.hops, 0);
    assert!(view.path.is_empty());

    let mut params = path_params("A", "C");
    params.max_hops = Some(9);
    assert!(tools.path(params).is_err());

    let mut params = path_params("A", "C");
    params.predicates = Some(vec!["depends_on".to_string()]);
    assert!(tools.path(params).unwrap().found);
    let mut params = path_params("A", "C");
    params.predicates = Some(vec!["fixes".to_string()]);
    assert!(!tools.path(params).unwrap().found);

    let mut params = path_params("A", "C");
    params.min_confidence = Some(0.6);
    assert!(tools.path(params).unwrap().found);
    let mut params = path_params("A", "C");
    params.min_confidence = Some(0.7);
    assert!(!tools.path(params).unwrap().found);
    let mut params = path_params("A", "C");
    params.min_confidence = Some(1.5);
    assert!(tools.path(params).is_err());
}

#[test]
fn path_does_not_cross_scopes_under_exact_walk() {
    let dir = tempfile::tempdir().unwrap();
    let tools = seeded(dir.path());

    let view = tools.path(path_params("A", "D")).unwrap();
    assert!(view.found, "unscoped path crosses proj/a into proj/b");
    assert_eq!(view.hops, 3);
    assert_eq!(view.scope_walk, "any");

    let mut params = path_params("A", "D");
    params.scope = Some("proj/a".to_string());
    params.scope_walk = Some("exact".to_string());
    let view = tools.path(params).unwrap();
    assert!(!view.found);
    assert_eq!(
        (view.scope.as_str(), view.scope_walk.as_str()),
        ("proj/a", "exact")
    );

    let mut params = path_params("A", "C");
    params.scope = Some("proj/a".to_string());
    params.scope_walk = Some("exact".to_string());
    assert!(tools.path(params).unwrap().found);
}

#[test]
fn path_step_carries_core_claim_id_from_add_triple() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();
    let written = tools
        .add_triple(AddTripleParams {
            subject: "M".to_string(),
            predicate: "depends_on".to_string(),
            object: "N".to_string(),
            confidence: Some(0.9),
            source: "graph-tools-test".to_string(),
            scope: None,
        })
        .unwrap();

    let view = tools.path(path_params("M", "N")).unwrap();
    assert!(view.found);
    assert_eq!(view.path.len(), 1);
    assert_eq!(view.path[0].claim_id, Some(written.triple_id));
}
