use aver_eval::tuning::{TuningObservation, TuningSearchSpace, suggest_next};

#[test]
fn bayesian_tuner_prefers_promising_untried_retrieval_neighborhood() {
    let space = TuningSearchSpace {
        top_k_min: 12,
        top_k_max: 20,
        alpha_min: 0.4,
        alpha_max: 0.8,
        alpha_steps: 5,
        exploration: 0.05,
    };
    let observations = vec![
        TuningObservation {
            top_k: 12,
            alpha: 0.4,
            metric: 50.0,
        },
        TuningObservation {
            top_k: 16,
            alpha: 0.6,
            metric: 63.0,
        },
        TuningObservation {
            top_k: 20,
            alpha: 0.8,
            metric: 54.0,
        },
    ];

    let suggestion = suggest_next(&space, &observations).unwrap();

    assert_ne!((suggestion.top_k, suggestion.alpha), (16, 0.6));
    assert!((14..=18).contains(&suggestion.top_k));
    assert!((0.5..=0.7).contains(&suggestion.alpha));
}

#[test]
fn bayesian_tuner_returns_safe_default_without_observations() {
    let suggestion = suggest_next(&TuningSearchSpace::default(), &[]).unwrap();

    assert_eq!(suggestion.top_k, 16);
    assert_eq!(suggestion.alpha, 0.65);
}
