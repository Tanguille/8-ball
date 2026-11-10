use domain::{
    config::BackendKind,
    question::Kind,
    verdict::{self, Bands, Example},
};

const fn noul() -> Kind {
    Kind::Noul { criteria: None }
}
fn example(probabilities: f64, label: &str) -> Example {
    Example {
        probs: vec![probabilities, 1.0 - probabilities],
        label: label.into(),
    }
}
#[test]
fn separable_noul() {
    let examples: Vec<_> = (0_u32..24)
        .map(|i| example(0.80 + 0.19 * f64::from(i) / 23.0, "yes"))
        .chain((0_u32..24).map(|i| example(0.01 + 0.19 * f64::from(i) / 23.0, "no")))
        .collect();
    let (bands, report) = verdict::calibrate(&noul(), &examples, 0.95, None).unwrap();
    let Bands::Noul { yes_at, no_at } = bands else {
        panic!("noul bands")
    };
    assert!((yes_at - 0.80).abs() < 1e-9);
    assert!((no_at - 0.20).abs() < 1e-9);
    assert!((report.coverage - 1.0).abs() < 1e-9);
    assert!((report.precision - 1.0).abs() < 1e-9);
    assert_eq!(report.decided, 48);
}
#[test]
fn too_few_examples() {
    let examples: Vec<_> = (0..5)
        .map(|_| example(0.9, "yes"))
        .chain((0..5).map(|_| example(0.1, "no")))
        .collect();
    let (bands, report) = verdict::calibrate(&noul(), &examples, 0.95, None).unwrap();
    let Bands::Noul { yes_at, no_at } = bands else {
        panic!("noul bands")
    };
    assert!((yes_at - 1.01).abs() < 1e-9);
    assert!((no_at + 0.01).abs() < 1e-9);
    assert!(report.warnings.len() >= 2);
    assert_eq!(report.decided, 0);
}
#[test]
fn bad_target() {
    for target in [0.4, 1.0] {
        assert!(verdict::calibrate(&noul(), &[], target, None).is_err());
    }
}
#[test]
fn overlap_gives_unsure() {
    assert_eq!(
        verdict::verdict(
            &noul(),
            &[0.5, 0.5],
            Bands::Noul {
                yes_at: 0.3,
                no_at: 0.7
            }
        ),
        "unsure"
    );
}
#[test]
fn choice_threshold() {
    let kind = Kind::Choice {
        criteria: [("a".into(), String::new()), ("b".into(), String::new())].into(),
    };
    let examples: Vec<_> = (0..20)
        .map(|_| example(0.9, "a"))
        .chain((0..10).map(|_| example(0.55, "b")))
        .collect();
    let (bands, report) = verdict::calibrate(&kind, &examples, 0.95, None).unwrap();
    let Bands::Level { min_confidence } = bands else {
        panic!("level bands")
    };
    assert!(min_confidence > 0.1 && min_confidence < 0.9);
    assert_eq!(report.decided, 20);
    assert_eq!(report.correct, 20);
}
#[test]
fn question_hash_changes() {
    let question = domain::question::Question {
        id: "first".into(),
        instructions: "Greeting?".into(),
        kind: noul(),
    };
    let mut other = question.clone();
    other.id = "second".into();
    assert_eq!(
        verdict::question_hash(&question),
        verdict::question_hash(&other)
    );
    other.instructions = "Storage risk?".into();
    assert_ne!(
        verdict::question_hash(&question),
        verdict::question_hash(&other)
    );
    assert_eq!(verdict::short_hash("hello"), "2cf24dba5fb0a30e");
}
#[test]
fn calibration_id_changes_with_fingerprint() {
    assert_ne!(
        verdict::calibration_id("q", BackendKind::Systemone, "file@b1"),
        verdict::calibration_id("q", BackendKind::Systemone, "file@b2")
    );
}
#[test]
fn explore_draw_is_deterministic() {
    let first = verdict::explore_draw("question", "state one");
    assert!((first - verdict::explore_draw("question", "state one")).abs() < f64::EPSILON);
    assert!((0.0..1.0).contains(&first));
    let second = verdict::explore_draw("question", "state two");
    assert!((0.0..1.0).contains(&second));
    assert!((first - second).abs() > f64::EPSILON);
}
#[test]
fn calibration_key_separates_ad_hoc() {
    assert_eq!(verdict::calibration_key("abc", Some("s")), "abc");
    assert_ne!(verdict::calibration_key("abc", None), "abc");
}
#[test]
fn merge_keeps_previous_side_without_evidence() {
    let examples: Vec<_> = (0..25).map(|_| example(0.97, "yes")).collect();
    let previous = Bands::Noul {
        yes_at: 0.85,
        no_at: 0.15,
    };
    let (bands, report) = verdict::calibrate(&noul(), &examples, 0.95, Some(previous)).unwrap();
    let Bands::Noul { yes_at, no_at } = bands else {
        panic!("noul bands")
    };
    assert!((yes_at - 0.97).abs() < 1e-9);
    assert!((no_at - 0.15).abs() < 1e-9);
    assert_eq!(report.decided, 25);
}
#[test]
fn merge_falls_back_to_defaults() {
    let previous = Bands::Noul {
        yes_at: 1.01,
        no_at: -0.01,
    };
    let (bands, _) = verdict::calibrate(&noul(), &[], 0.95, Some(previous)).unwrap();
    assert_eq!(bands, Bands::default_for(&noul()));
}
#[test]
fn merge_crossing_returns_previous() {
    // The no side decides at 0.9, above the previous yes_at of 0.85.
    let examples: Vec<_> = (0..25).map(|_| example(0.9, "no")).collect();
    let previous = Bands::Noul {
        yes_at: 0.85,
        no_at: 0.15,
    };
    let (bands, _) = verdict::calibrate(&noul(), &examples, 0.95, Some(previous)).unwrap();
    assert_eq!(bands, previous);
}
#[test]
fn merge_level_keeps_previous() {
    let kind = Kind::Choice {
        criteria: [("a".into(), String::new()), ("b".into(), String::new())].into(),
    };
    let previous = Bands::Level {
        min_confidence: 0.7,
    };
    let (bands, _) = verdict::calibrate(&kind, &[], 0.95, Some(previous)).unwrap();
    assert_eq!(bands, previous);
}
