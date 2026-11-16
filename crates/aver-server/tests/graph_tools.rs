//! Roadmap Phase 5: `communities` over the core graph.

use aver_server::tools::{AddTripleParams, AverTools, CommunitiesParams};

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
                provenance: None,
                object_kind: None,
                event_id: None,
                strict_atomic: None,
                subject: subject.to_string(),
                predicate: "depends_on".to_string(),
                object: object.to_string(),
                confidence: Some(confidence),
                source: "graph-tools-test".to_string(),
                scope: Some(scope.to_string()),
                agent_id: None,
                agent_kind: Some("HUMAN".to_string()),
                context: None,
                valid_from: None,
                valid_until: None,
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
