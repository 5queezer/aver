//! Opt-in bounded graph-first recall: deterministic routing, at most
//! `MAX_ANCHORS` anchors, an edge budget on expansion, and source events
//! only for the claims it selects. `None` tells the caller to fall back to
//! hybrid recall.

use std::path::Path;

use aver_core::graph_first::{
    GraphFirstFallback, GraphFirstOptions, MAX_ANCHORS, MAX_EDGES, QueryRoute, route_query,
};
use aver_core::retrieval::{DEFAULT_RECENCY_TAU_SECS, HybridWeights};
use aver_core::{ClaimWrite, EventWrite, RecallFilters, Store, replay};

const DAY: i64 = 86_400;
const NOW: i64 = 1_793_491_200;

fn open(dir: &Path) -> Store {
    Store::open(dir).unwrap()
}

fn add(store: &Store, subject: &str, predicate: &str, object: &str) -> i64 {
    store
        .add_claim(ClaimWrite::new(subject, predicate, object, "test"))
        .unwrap()
}

fn recall(store: &Store, query: &str) -> aver_core::graph_first::GraphFirstRecall {
    store
        .recall_graph_first(
            query,
            &RecallFilters::default(),
            &GraphFirstOptions::default(),
            NOW,
        )
        .unwrap()
        .expect("graph-first should find an anchor")
}

fn claim_ids(result: &aver_core::graph_first::GraphFirstRecall) -> Vec<i64> {
    result.facts.iter().map(|fact| fact.claim.id).collect()
}

#[test]
fn routes_queries_by_word_cues_in_a_fixed_precedence() {
    assert_eq!(
        route_query("How many services does Billing use?", 1),
        QueryRoute::Count
    );
    assert_eq!(
        route_query("When did Billing last change?", 1),
        QueryRoute::Temporal
    );
    assert_eq!(
        route_query("What is the latest Billing owner", 1),
        QueryRoute::Temporal
    );
    assert_eq!(
        route_query("What do Billing and Search both use", 2),
        QueryRoute::Intersection
    );
    // An intersection cue needs two anchors to mean anything.
    assert_eq!(
        route_query("What do Billing and Search both use", 1),
        QueryRoute::Single
    );
    assert_eq!(
        route_query("List the services Billing uses", 1),
        QueryRoute::List
    );
    assert_eq!(
        route_query("Which services does Billing use", 1),
        QueryRoute::List
    );
    assert_eq!(route_query("Who owns Billing", 1), QueryRoute::Single);
    // Cues are whole words: "lastly" and "listing" are not cues.
    assert_eq!(route_query("Billing lastly listing", 1), QueryRoute::Single);
}

#[test]
fn anchors_on_entities_named_in_the_query_and_expands_from_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let dep = add(&store, "PaymentGateway", "depends_on", "StripeSDK");
    let owner = add(&store, "PaymentGateway", "owned_by", "team_payments");
    let unrelated = add(&store, "SearchIndex", "depends_on", "Tantivy");

    let result = recall(&store, "Who owns the PaymentGateway?");

    assert_eq!(result.route, QueryRoute::Single);
    assert_eq!(result.anchors, vec!["PaymentGateway".to_string()]);
    let ids = claim_ids(&result);
    assert!(ids.contains(&owner) && ids.contains(&dep), "{ids:?}");
    assert!(!ids.contains(&unrelated));
    // The predicate the question asks about ranks first.
    assert_eq!(ids[0], owner);
    assert!(
        result
            .facts
            .iter()
            .all(|fact| fact.anchor == "PaymentGateway" && fact.hop == 1)
    );
}

#[test]
fn returns_none_when_no_entity_matches_so_the_caller_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "PaymentGateway", "depends_on", "StripeSDK");

    let result = store
        .recall_graph_first(
            "what is the weather like",
            &RecallFilters::default(),
            &GraphFirstOptions::default(),
            NOW,
        )
        .unwrap();

    assert!(matches!(result, Err(GraphFirstFallback::NoAnchor)));
}

#[test]
fn caps_anchors_at_fifteen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for i in 0..(MAX_ANCHORS + 5) {
        add(&store, &format!("cache_{i}"), "depends_on", "redis");
    }

    let result = recall(&store, "what does cache depend on");

    assert_eq!(result.anchors.len(), MAX_ANCHORS);
    assert_eq!(result.budget.anchors, MAX_ANCHORS);
    assert_eq!(result.budget.max_anchors, MAX_ANCHORS);
}

#[test]
fn stops_expanding_at_the_edge_budget_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for i in 0..(MAX_EDGES + 20) {
        add(&store, "Monolith", "depends_on", &format!("lib_{i}"));
    }

    let result = recall(&store, "what does Monolith depend on");

    assert_eq!(result.budget.edges_visited, MAX_EDGES);
    assert_eq!(result.budget.max_edges, MAX_EDGES);
    assert!(result.budget.truncated);
    assert!(result.subgraph.edges.len() <= MAX_EDGES);
}

#[test]
fn hops_bound_the_expansion() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let near = add(&store, "Billing", "depends_on", "ledger");
    let far = add(&store, "ledger", "depends_on", "postgres");

    let one_hop = store
        .recall_graph_first(
            "Billing",
            &RecallFilters::default(),
            &GraphFirstOptions {
                hops: 1,
                ..GraphFirstOptions::default()
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    let two_hops = recall(&store, "Billing");

    let one: Vec<i64> = one_hop.subgraph.edges.iter().map(|c| c.id).collect();
    assert_eq!(one, vec![near]);
    assert!(two_hops.subgraph.edges.iter().any(|c| c.id == far));
    let far_fact = two_hops.facts.iter().find(|f| f.claim.id == far).unwrap();
    assert_eq!(far_fact.hop, 2);
}

#[test]
fn returns_source_events_only_for_selected_claims() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let chosen_event = store
        .record_event(EventWrite::new(
            "s1",
            "decision",
            "picked Stripe for cards",
            "test",
        ))
        .unwrap();
    let other_event = store
        .record_event(EventWrite::new("s1", "decision", "picked Tantivy", "test"))
        .unwrap();
    store
        .add_claim(
            ClaimWrite::new("PaymentGateway", "depends_on", "StripeSDK", "test")
                .event_id(chosen_event),
        )
        .unwrap();
    store
        .add_claim(
            ClaimWrite::new("SearchIndex", "depends_on", "Tantivy", "test").event_id(other_event),
        )
        .unwrap();

    let result = recall(&store, "what does PaymentGateway depend on");

    let event_ids: Vec<i64> = result.events.iter().map(|e| e.id).collect();
    assert_eq!(event_ids, vec![chosen_event]);
    assert_eq!(result.events[0].payload, "picked Stripe for cards");
}

#[test]
fn intersection_keeps_claims_that_connect_both_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let billing_redis = add(&store, "Billing", "depends_on", "redis");
    let search_redis = add(&store, "Search", "depends_on", "redis");
    let billing_only = add(&store, "Billing", "depends_on", "ledger");
    let search_only = add(&store, "Search", "depends_on", "tantivy");

    let result = recall(&store, "What do Billing and Search both depend on?");

    assert_eq!(result.route, QueryRoute::Intersection);
    let ids = claim_ids(&result);
    assert!(
        ids.contains(&billing_redis) && ids.contains(&search_redis),
        "{ids:?}"
    );
    assert!(
        !ids.contains(&billing_only) && !ids.contains(&search_only),
        "{ids:?}"
    );
}

#[test]
fn count_reports_how_many_matching_claims_the_anchor_has() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for lib in ["serde", "tokio", "rusqlite"] {
        add(&store, "Aver", "depends_on", lib);
    }
    add(&store, "Aver", "owned_by", "christian");

    let result = recall(&store, "How many crates does Aver depend on?");

    assert_eq!(result.route, QueryRoute::Count);
    assert_eq!(result.count, Some(3));
    assert_eq!(result.facts.len(), 3);
    assert!(
        result
            .facts
            .iter()
            .all(|f| f.claim.predicate == "depends_on")
    );
}

#[test]
fn list_returns_every_matching_claim_beyond_top_k() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for i in 0..8 {
        add(&store, "Aver", "depends_on", &format!("crate_{i}"));
    }
    add(&store, "Aver", "owned_by", "christian");

    let result = recall(&store, "List everything Aver depends on");

    assert_eq!(result.route, QueryRoute::List);
    assert_eq!(result.facts.len(), 8);
    assert!(
        result
            .facts
            .iter()
            .all(|f| f.claim.predicate == "depends_on")
    );
}

fn claim_line(ts: i64, claim_id: i64, subject: &str, predicate: &str, object: &str) -> String {
    serde_json::json!({
        "kind": "add_claim", "ts": ts, "claim_id": claim_id,
        "subject": subject, "predicate": predicate, "object": object,
        "source": format!("src-{claim_id}"), "agent_id": "local", "agent_kind": "HUMAN",
        "confidence": 0.95, "provenance": "USER_ASSERTED", "scope": "global",
    })
    .to_string()
}

fn seeded(dir: &Path, lines: &[String]) -> Store {
    std::fs::write(dir.join("log.jsonl"), lines.join("\n") + "\n").unwrap();
    replay(dir, false).unwrap();
    Store::open(dir).unwrap()
}

#[test]
fn temporal_orders_the_bounded_set_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(
        dir.path(),
        &[
            claim_line(NOW - 30 * DAY, 1, "Billing", "depends_on", "mysql"),
            claim_line(NOW - DAY, 2, "Billing", "depends_on", "postgres"),
            claim_line(NOW - 10 * DAY, 3, "Billing", "owned_by", "team_a"),
        ],
    );

    let result = recall(&store, "What did Billing depend on most recently?");

    assert_eq!(result.route, QueryRoute::Temporal);
    let ids = claim_ids(&result);
    assert_eq!(ids[0], 2, "{ids:?}");
    let ts: Vec<i64> = result.facts.iter().map(|f| f.ts).collect();
    assert!(ts.windows(2).all(|w| w[0] >= w[1]), "{ts:?}");
}

#[test]
fn recency_reranks_only_inside_the_bounded_candidate_set() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(
        dir.path(),
        &[
            claim_line(NOW - 400 * DAY, 1, "Billing", "depends_on", "mysql"),
            claim_line(NOW - 400 * DAY, 2, "Billing", "depends_on", "postgres"),
            claim_line(NOW - DAY, 3, "Billing", "depends_on", "sqlite"),
            // Fresh but outside the anchor's neighbourhood.
            claim_line(NOW, 4, "Unrelated", "depends_on", "sqlite_fork"),
        ],
    );
    let with_recency = |weight: f64| GraphFirstOptions {
        weights: HybridWeights::default()
            .with_recency(weight, DEFAULT_RECENCY_TAU_SECS)
            .unwrap(),
        top_k: 2,
        ..GraphFirstOptions::default()
    };

    let plain = store
        .recall_graph_first(
            "Billing depends on",
            &RecallFilters::default(),
            &with_recency(0.0),
            NOW,
        )
        .unwrap()
        .unwrap();
    let recent = store
        .recall_graph_first(
            "Billing depends on",
            &RecallFilters::default(),
            &with_recency(0.5),
            NOW,
        )
        .unwrap()
        .unwrap();

    assert_eq!(claim_ids(&plain), vec![1, 2]);
    assert_eq!(claim_ids(&recent)[0], 3);
    assert!(!claim_ids(&recent).contains(&4));
}

#[test]
fn honours_scope_filters_during_expansion() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let inside = store
        .add_claim(ClaimWrite::new("Billing", "depends_on", "ledger", "test").scope("proj/a"))
        .unwrap();
    store
        .add_claim(ClaimWrite::new("Billing", "depends_on", "secret", "test").scope("proj/b"))
        .unwrap();

    let filters = RecallFilters {
        scope: "proj/a".to_string(),
        scope_walk: aver_core::ScopeWalk::Exact,
        ..RecallFilters::default()
    };
    let result = store
        .recall_graph_first("Billing", &filters, &GraphFirstOptions::default(), NOW)
        .unwrap()
        .unwrap();

    let ids: Vec<i64> = result.subgraph.edges.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![inside]);
}

#[test]
fn reports_truncation_when_already_seen_edges_fill_the_last_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // The hub's own edges leave one slot; the first leaf repeats the hub
    // edge and has two new ones, so one of them must be cut.
    for i in 0..(MAX_EDGES - 1) {
        add(&store, "Hub", "depends_on", &format!("leaf_{i}"));
    }
    add(&store, "leaf_0", "depends_on", "deep_a");
    add(&store, "leaf_0", "depends_on", "deep_b");

    let result = recall(&store, "Hub");

    assert_eq!(result.budget.edges_visited, MAX_EDGES);
    assert!(result.budget.truncated);
}

// Review findings on the first eval run (P1.1-P1.3, P2.4, P2.6, P3.7).

#[test]
fn versioned_entities_anchor_on_their_words_not_digit_fragments() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(
        dir.path(),
        &[
            claim_line(
                NOW - 300 * DAY,
                1,
                "checkout-service",
                "uses",
                "stripe-api-v1",
            ),
            claim_line(
                NOW - 30 * DAY,
                2,
                "checkout-service",
                "uses",
                "stripe-api-v3",
            ),
            claim_line(
                NOW - 5 * DAY,
                3,
                "api-gateway",
                "depends_on",
                "checkout-service",
            ),
        ],
    );

    let result = recall(
        &store,
        "What is the latest Stripe API version used by checkout-service?",
    );

    assert!(
        result.anchors.contains(&"stripe-api-v3".to_string()),
        "{:?}",
        result.anchors
    );
    assert!(
        result.anchors.contains(&"stripe-api-v1".to_string()),
        "{:?}",
        result.anchors
    );
    assert!(
        !result.anchors.contains(&"api-gateway".to_string()),
        "{:?}",
        result.anchors
    );
    assert_eq!(claim_ids(&result)[0], 2);
}

#[test]
fn content_words_match_claim_subjects_and_cue_words_are_not_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(
        dir.path(),
        &[
            claim_line(
                NOW - 10 * DAY,
                1,
                "incident-2026-09-14",
                "concerns",
                "checkout-service",
            ),
            claim_line(
                NOW - 9 * DAY,
                2,
                "rollback-42",
                "concerns",
                "checkout-service",
            ),
        ],
    );

    let result = recall(
        &store,
        "What was the most recent incident affecting checkout-service?",
    );

    assert_eq!(result.route, QueryRoute::Temporal);
    assert_eq!(claim_ids(&result)[0], 1, "{:?}", claim_ids(&result));
}

#[test]
fn inflected_query_words_match_inflected_predicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "checkout-service", "owned_by", "team_pay");
    let uses = add(&store, "checkout-service", "uses", "redis");

    let result = recall(&store, "What was checkout-service using?");

    assert_eq!(claim_ids(&result)[0], uses);
}

#[test]
fn count_and_list_keep_claims_touching_the_primary_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for svc in ["auth-service", "billing-service", "search-service"] {
        add(&store, svc, "owned_by", "platform-team");
    }
    for svc in ["mail-service", "media-service"] {
        add(&store, svc, "owned_by", "growth-team");
    }
    add(&store, "auth-service", "depends_on", "postgres");
    add(&store, "billing-service", "depends_on", "postgres");
    add(&store, "auth-service", "depends_on", "kafka");
    add(&store, "mail-service", "depends_on", "kubernetes");

    let owned = recall(&store, "How many services does platform-team own?");
    let deps = recall(&store, "How many services depend on postgres?");

    assert_eq!(owned.count, Some(3), "{:?}", claim_ids(&owned));
    assert_eq!(deps.count, Some(2), "{:?}", claim_ids(&deps));
}

#[test]
fn list_matches_predicates_through_the_ontology() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let depends = add(&store, "orders-service", "depends_on", "kafka");
    // `uses` is declared a child of `depends_on`.
    let uses = add(&store, "notification-service", "uses", "kafka");
    add(&store, "kafka", "owned_by", "infra-team");

    let result = recall(&store, "Which services depend on kafka?");

    assert_eq!(result.route, QueryRoute::List);
    let mut ids = claim_ids(&result);
    ids.sort_unstable();
    assert_eq!(ids, vec![depends, uses]);
}

#[test]
fn fallback_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "PaymentGateway", "depends_on", "StripeSDK");

    let outcome = store
        .recall_graph_first(
            "what is the weather like",
            &RecallFilters::default(),
            &GraphFirstOptions::default(),
            NOW,
        )
        .unwrap();

    assert!(matches!(outcome, Err(GraphFirstFallback::NoAnchor)));
}

#[test]
fn route_cue_words_never_match_claim_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // "list" and "all" are route cues; they must not pull this claim in.
    store
        .add_claim(
            ClaimWrite::new("Billing", "concerns", "list of all invoices", "test")
                .object_kind(aver_core::ObjectKind::Literal),
        )
        .unwrap();
    let owner = add(&store, "Billing", "owned_by", "team_a");

    let result = recall(&store, "List all owners of Billing");

    assert_eq!(result.route, QueryRoute::List);
    assert_eq!(claim_ids(&result), vec![owner]);
}

// CodeRabbit findings on bb1e07f.

#[test]
fn an_entity_named_on_its_own_survives_subset_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "billing", "depends_on", "redis");
    add(&store, "billing-worker", "depends_on", "redis");

    let both = recall(&store, "What do billing and billing-worker both depend on?");
    let worker_only = recall(&store, "What does billing-worker depend on?");

    assert!(
        both.anchors.contains(&"billing".to_string()),
        "{:?}",
        both.anchors
    );
    assert!(both.anchors.contains(&"billing-worker".to_string()));
    assert_eq!(both.route, QueryRoute::Intersection);
    // Inside "billing-worker" the word "billing" names billing-worker only.
    assert_eq!(worker_only.anchors, vec!["billing-worker".to_string()]);
}

#[test]
fn count_with_unmatched_content_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "Billing", "owned_by", "team_a");

    let result = recall(&store, "How many zebras does Billing have?");

    assert_eq!(result.route, QueryRoute::Count);
    assert_eq!(result.count, Some(0));
    assert!(result.facts.is_empty());
}

#[test]
fn earliest_questions_sort_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(
        dir.path(),
        &[
            claim_line(NOW - 30 * DAY, 1, "Billing", "depends_on", "mysql"),
            claim_line(NOW - DAY, 2, "Billing", "depends_on", "postgres"),
        ],
    );

    for query in [
        "What did Billing depend on first?",
        "What is the oldest thing Billing depends on?",
        "What was the earliest dependency of Billing?",
    ] {
        let result = recall(&store, query);
        assert_eq!(result.route, QueryRoute::Temporal, "{query}");
        assert_eq!(claim_ids(&result)[0], 1, "{query}");
    }
    assert_eq!(
        claim_ids(&recall(&store, "What does Billing depend on latest?"))[0],
        2
    );
}

#[test]
fn temporal_constraints_relative_to_an_event_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    add(&store, "Billing", "depends_on", "postgres");

    for query in [
        "What did Billing depend on before the migration?",
        "What did Billing depend on after the outage?",
        "What has Billing depended on since the rewrite?",
        "What did Billing depend on until the cutover?",
    ] {
        let outcome = store
            .recall_graph_first(
                query,
                &RecallFilters::default(),
                &GraphFirstOptions::default(),
                NOW,
            )
            .unwrap();
        assert!(
            matches!(outcome, Err(GraphFirstFallback::TemporalConstraint)),
            "{query}: {outcome:?}"
        );
    }
}
