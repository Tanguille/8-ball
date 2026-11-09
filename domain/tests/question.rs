use serde_json::json;

use domain::question::{Kind, Question};

#[test]
fn choice_roundtrip_keeps_order() {
    let input = r#"{"id":"q","instructions":"x","type":"choice","criteria":{"b":"","a":"desc"}}"#;
    let question: Question = serde_json::from_str(input).unwrap();
    assert_eq!(question.kind.labels(), ["b", "a"]);
    assert_eq!(serde_json::to_string(&question).unwrap(), input);
    assert_eq!(question.kind.type_name(), "choice");
}

#[test]
fn noul_without_criteria_parses() {
    let question: Question =
        serde_json::from_value(json!({"id":"q","instructions":"x","type":"noul"})).unwrap();
    assert_eq!(question.kind, Kind::Noul { criteria: None });
    assert_eq!(question.kind.labels(), ["yes", "no"]);
    assert_eq!(question.kind.type_name(), "noul");
    question.validate().unwrap();
}

#[test]
fn score_labels() {
    let question: Question = serde_json::from_value(
        json!({"id":"q","instructions":"x","type":"score","criteria":["low","mid","high"]}),
    )
    .unwrap();
    assert_eq!(question.kind.labels(), ["0", "1", "2"]);
    assert_eq!(question.kind.type_name(), "score");
    question.validate().unwrap();
}

#[test]
fn validate_rejects() {
    for (kind, message) in [
        (
            Kind::Choice {
                criteria: [("a".into(), String::new())].into(),
            },
            "options",
        ),
        (
            Kind::Score {
                criteria: vec!["low".into()],
            },
            "levels",
        ),
        (
            Kind::Score {
                criteria: vec!["x".into(); 11],
            },
            "levels",
        ),
    ] {
        let question = Question {
            id: "q".into(),
            instructions: "x".into(),
            kind,
        };
        assert!(
            question
                .validate()
                .unwrap_err()
                .to_string()
                .contains(message)
        );
    }
    let mut question = Question {
        id: "q".into(),
        instructions: String::new(),
        kind: Kind::Noul { criteria: None },
    };
    assert!(
        question
            .validate()
            .unwrap_err()
            .to_string()
            .contains("instructions")
    );
    question.instructions = "x".into();
    assert!(
        domain::question::validate_all(&[question.clone(), question])
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert!(
        domain::question::validate_all(&[])
            .unwrap_err()
            .to_string()
            .contains("no questions")
    );
}
