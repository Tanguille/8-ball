use eightball::question::{Kind, Question, validate_all};
use serde_json::json;

#[test]
fn choice_roundtrip_keeps_order() {
    let input = r#"{"id":"q","instructions":"x","type":"choice","criteria":{"b":"","a":"desc"}}"#;
    let q: Question = serde_json::from_str(input).unwrap();
    assert_eq!(q.kind.labels(), ["b", "a"]);
    assert_eq!(serde_json::to_string(&q).unwrap(), input);
    assert_eq!(q.kind.type_name(), "choice");
}

#[test]
fn noul_without_criteria_parses() {
    let q: Question =
        serde_json::from_value(json!({"id":"q","instructions":"x","type":"noul"})).unwrap();
    assert_eq!(q.kind, Kind::Noul { criteria: None });
    assert_eq!(q.kind.labels(), ["yes", "no"]);
    assert_eq!(q.kind.type_name(), "noul");
    q.validate().unwrap();
}

#[test]
fn score_labels() {
    let q: Question = serde_json::from_value(
        json!({"id":"q","instructions":"x","type":"score","criteria":["low","mid","high"]}),
    )
    .unwrap();
    assert_eq!(q.kind.labels(), ["0", "1", "2"]);
    assert_eq!(q.kind.type_name(), "score");
    q.validate().unwrap();
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
        let q = Question {
            id: "q".into(),
            instructions: "x".into(),
            kind,
        };
        assert!(q.validate().unwrap_err().to_string().contains(message));
    }
    let mut q = Question {
        id: "q".into(),
        instructions: String::new(),
        kind: Kind::Noul { criteria: None },
    };
    assert!(
        q.validate()
            .unwrap_err()
            .to_string()
            .contains("instructions")
    );
    q.instructions = "x".into();
    assert!(
        validate_all(&[q.clone(), q])
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert!(
        validate_all(&[])
            .unwrap_err()
            .to_string()
            .contains("no questions")
    );
}
