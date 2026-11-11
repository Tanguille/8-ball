use domain::{
    config::BackendKind,
    question::Kind,
    verdict::{self, Bands},
};

#[test]
fn choice_confidence_readme() {
    assert!((verdict::confidence_choice(&[0.9998, 0.0001, 0.0001]) - 0.9997).abs() < 1e-3);
}
#[test]
fn score_confidence_readme() {
    assert!((verdict::confidence_score(&[0.0023, 0.116, 0.6753, 0.2064]) - 0.673).abs() < 1e-3);
}
#[test]
fn uniform_is_zero() {
    assert!(verdict::confidence_choice(&[0.5, 0.5]).abs() < f64::EPSILON);
}
#[test]
fn noul_default_bands() {
    let kind = Kind::Noul { criteria: None };
    for (probabilities, want) in [
        (0.9, "yes"),
        (0.85, "yes"),
        (0.5, "unsure"),
        (0.15, "no"),
        (0.05, "no"),
    ] {
        assert_eq!(
            verdict::verdict(
                &kind,
                &[probabilities, 1.0 - probabilities],
                Bands::default_for(&kind, BackendKind::Systemone)
            ),
            want
        );
    }
}
#[test]
fn choice_default_bands() {
    let kind = Kind::Choice {
        criteria: [
            ("a".into(), String::new()),
            ("b".into(), String::new()),
            ("c".into(), String::new()),
        ]
        .into(),
    };
    assert!((verdict::confidence_choice(&[0.9, 0.05, 0.05]) - 0.85).abs() < 1e-9);
    assert_eq!(
        verdict::verdict(
            &kind,
            &[0.9, 0.05, 0.05],
            Bands::default_for(&kind, BackendKind::Systemone)
        ),
        "a"
    );
    assert_eq!(
        verdict::verdict(
            &kind,
            &[0.4, 0.35, 0.25],
            Bands::default_for(&kind, BackendKind::Systemone)
        ),
        "unsure"
    );
}
#[test]
fn score_verdict_is_mode_index() {
    let kind = Kind::Score {
        criteria: vec!["low".into(), "mid".into(), "high".into()],
    };
    assert_eq!(
        verdict::verdict(
            &kind,
            &[0.1, 0.8, 0.1],
            Bands::default_for(&kind, BackendKind::Systemone)
        ),
        "1"
    );
    assert!((verdict::expected_score(&[0.1, 0.8, 0.1]) - 1.0).abs() < 1e-9);
}

#[test]
fn uncalibrated_rerank_noul_is_unsure() {
    // A reranker scores topic match, not the answer, so even extreme scores must not decide.
    let kind = Kind::Noul { criteria: None };
    for p_yes in [0.0, 0.001, 0.5, 0.999, 1.0] {
        assert_eq!(
            verdict::verdict(
                &kind,
                &[p_yes, 1.0 - p_yes],
                Bands::default_for(&kind, BackendKind::Rerank)
            ),
            "unsure"
        );
    }
}
