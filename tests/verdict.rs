use eightball::question::Kind;
use eightball::verdict::{Bands, confidence_choice, confidence_score, expected_score, verdict};

#[test]
fn choice_confidence_readme() {
    assert!((confidence_choice(&[0.9998, 0.0001, 0.0001]) - 0.9997).abs() < 1e-3);
}
#[test]
fn score_confidence_readme() {
    assert!((confidence_score(&[0.0023, 0.116, 0.6753, 0.2064]) - 0.673).abs() < 1e-3);
}
#[test]
fn uniform_is_zero() {
    assert!(confidence_choice(&[0.5, 0.5]).abs() < f64::EPSILON);
}
#[test]
fn noul_default_bands() {
    let kind = Kind::Noul { criteria: None };
    for (p, want) in [
        (0.9, "yes"),
        (0.85, "yes"),
        (0.5, "unsure"),
        (0.15, "no"),
        (0.05, "no"),
    ] {
        assert_eq!(
            verdict(
                &kind,
                &kind.labels(),
                &[p, 1.0 - p],
                Bands::default_for(&kind)
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
    assert!((confidence_choice(&[0.9, 0.05, 0.05]) - 0.85).abs() < 1e-9);
    assert_eq!(
        verdict(
            &kind,
            &kind.labels(),
            &[0.9, 0.05, 0.05],
            Bands::default_for(&kind)
        ),
        "a"
    );
    assert_eq!(
        verdict(
            &kind,
            &kind.labels(),
            &[0.4, 0.35, 0.25],
            Bands::default_for(&kind)
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
        verdict(
            &kind,
            &kind.labels(),
            &[0.1, 0.8, 0.1],
            Bands::default_for(&kind)
        ),
        "1"
    );
    assert!((expected_score(&[0.1, 0.8, 0.1]) - 1.0).abs() < 1e-9);
}
