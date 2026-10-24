//! ADR-0024 decision-model predicate normalization.

use aver_core::Store;
use aver_core::decision::{DecisionError, MAX_OPTIONS, MockDecisionClient};

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn options_for(store: &Store, predicate: &str) -> Vec<String> {
    let probe = MockDecisionClient::with_indices(vec![0]);
    store.normalize_predicate(predicate, &probe).unwrap();
    probe.received().remove(0).options
}

#[test]
fn canonical_predicate_skips_client() {
    let (_dir, store) = store();
    let client = MockDecisionClient::with_indices(vec![0]);
    assert_eq!(
        store.normalize_predicate("depends_on", &client).unwrap(),
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
        .normalize_predicate("needs", &client)
        .unwrap()
        .expect("suggestion");
    assert_eq!(suggestion.accepted, "depends_on");
}

#[test]
fn none_choice_returns_none() {
    let (_dir, store) = store();
    let none = options_for(&store, "needs").len() - 1;

    let client = MockDecisionClient::with_indices(vec![none]);
    assert_eq!(store.normalize_predicate("needs", &client).unwrap(), None);
}

#[test]
fn unavailable_falls_back_to_deterministic_suggestion() {
    let (_dir, store) = store();
    let client = MockDecisionClient::new(vec![Err(DecisionError::Unavailable)]);
    let suggestion = store
        .normalize_predicate("needs", &client)
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
