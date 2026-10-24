//! ADR-0024 decision-model predicate normalization.

use aver_core::Store;
use aver_core::decision::{DecisionError, MAX_OPTIONS, MockDecisionClient};

const FLOOR: f64 = 0.5;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn options_for(store: &Store, predicate: &str) -> Vec<String> {
    let probe = MockDecisionClient::with_indices(vec![0]);
    store
        .normalize_predicate(predicate, None, &probe, FLOOR)
        .unwrap();
    probe.received().remove(0).options
}

#[test]
fn canonical_predicate_skips_client() {
    let (_dir, store) = store();
    let client = MockDecisionClient::with_indices(vec![0]);
    assert_eq!(
        store
            .normalize_predicate("depends_on", None, &client, FLOOR)
            .unwrap(),
        None
    );
    assert!(client.received().is_empty());
}

#[test]
fn model_choice_maps_to_suggestion() {
    let (_dir, store) = store();
    let index = options_for(&store, "needs")
        .iter()
        .position(|o| o == "depends_on")
        .unwrap();

    let client = MockDecisionClient::with_indices(vec![index]);
    let suggestion = store
        .normalize_predicate("needs", None, &client, FLOOR)
        .unwrap()
        .expect("suggestion");
    assert_eq!(suggestion.accepted, "depends_on");
}

#[test]
fn none_choice_returns_none() {
    let (_dir, store) = store();
    let none = options_for(&store, "needs").len() - 1;

    let client = MockDecisionClient::with_indices(vec![none]);
    assert_eq!(
        store
            .normalize_predicate("needs", None, &client, FLOOR)
            .unwrap(),
        None
    );
}

#[test]
fn unavailable_falls_back_to_deterministic_suggestion() {
    let (_dir, store) = store();
    let client = MockDecisionClient::new(vec![Err(DecisionError::Unavailable)]);
    let suggestion = store
        .normalize_predicate("needs", None, &client, FLOOR)
        .unwrap()
        .expect("deterministic suggestion");
    assert_eq!(suggestion.accepted, "depends_on");
}

#[test]
fn option_list_is_stable_none_last_and_bounded() {
    let (_dir, store) = store();
    let options = options_for(&store, "needs");

    assert_eq!(options, options_for(&store, "needs"));
    assert_eq!(options.last().unwrap(), "none");
    assert!(options.len() <= MAX_OPTIONS);
    let vocabulary = &options[..options.len() - 1];
    let mut sorted = vocabulary.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(vocabulary, sorted);
    assert!(options.iter().any(|o| o == "depends_on"));
}

fn index_of(store: &Store, predicate: &str, option: &str) -> usize {
    options_for(store, predicate)
        .iter()
        .position(|o| o == option)
        .unwrap()
}

#[test]
fn options_carry_descriptions_and_state_carries_example() {
    let (_dir, store) = store();
    let client = MockDecisionClient::with_indices(vec![0]);
    store
        .normalize_predicate("needs", Some("PaymentGateway needs Redis"), &client, FLOOR)
        .unwrap();
    let received = client.received().remove(0);
    let descriptions = received.descriptions.expect("descriptions");
    assert_eq!(descriptions.len(), received.options.len());
    let depends_on = received
        .options
        .iter()
        .position(|o| o == "depends_on")
        .unwrap();
    assert!(
        descriptions[depends_on].starts_with("depends_on")
            && descriptions[depends_on]
                .ends_with(": subject requires, needs, or relies on the object"),
        "{}",
        descriptions[depends_on]
    );
    assert_eq!(
        descriptions.last().unwrap(),
        "no listed predicate fits this input"
    );
    assert!(received.question.contains("Pick none only if"));
    assert!(received.state.contains("predicate: needs"));
    assert!(received.state.contains("normalized: needs"));
    assert!(
        received
            .state
            .contains("example: PaymentGateway needs Redis")
    );
}

#[test]
fn floor_gates_model_choice() {
    let (_dir, store) = store();
    let fixes = index_of(&store, "needs", "fixes");
    let none = options_for(&store, "needs").len() - 1;

    // Below the floor the deterministic suggestion (depends_on) wins.
    let unsure = MockDecisionClient::with_choices(vec![(fixes, Some(0.46)), (none, Some(0.2))]);
    for _ in 0..2 {
        let suggestion = store
            .normalize_predicate("needs", None, &unsure, FLOOR)
            .unwrap()
            .expect("fallback suggestion");
        assert_eq!(suggestion.accepted, "depends_on");
    }

    let sure = MockDecisionClient::with_choices(vec![(fixes, Some(0.99)), (none, Some(0.85))]);
    let suggestion = store
        .normalize_predicate("needs", None, &sure, FLOOR)
        .unwrap()
        .expect("model suggestion");
    assert_eq!(suggestion.accepted, "fixes");
    assert_eq!(
        store
            .normalize_predicate("needs", None, &sure, FLOOR)
            .unwrap(),
        None
    );

    // Missing confidence fails the floor: the deterministic suggestion wins.
    let silent = MockDecisionClient::with_choices(vec![(fixes, None)]);
    assert_eq!(
        store
            .normalize_predicate("needs", None, &silent, FLOOR)
            .unwrap()
            .unwrap()
            .accepted,
        "depends_on"
    );
}

#[test]
fn vocabulary_predicate_named_none_gets_a_distinct_sentinel() {
    let (_dir, store) = store();
    // ADR-0018: a USER_ASSERTED claim auto-extends the vocabulary.
    store.add_claim("a", "none", "b", "s1").unwrap();

    let options = options_for(&store, "needs");
    assert_eq!(options.last().unwrap(), "none_of_the_above");
    let none_predicate = options.iter().position(|o| o == "none").unwrap();
    assert!(none_predicate < options.len() - 1);

    let client = MockDecisionClient::with_indices(vec![none_predicate, options.len() - 1]);
    let suggestion = store
        .normalize_predicate("needs", None, &client, FLOOR)
        .unwrap()
        .expect("the predicate `none` is suggestible");
    assert_eq!(suggestion.accepted, "none");
    assert_eq!(
        store
            .normalize_predicate("needs", None, &client, FLOOR)
            .unwrap(),
        None,
        "the sentinel still means no match"
    );
    let received = client.received();
    assert!(
        received[0]
            .question
            .contains("Pick none_of_the_above only if")
    );
    assert_eq!(
        received[0].descriptions.as_ref().unwrap().last().unwrap(),
        "no listed predicate fits this input"
    );
}
