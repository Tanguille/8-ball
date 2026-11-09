use serde_json::{Value, json};
use wiremock::{MockServer, ResponseTemplate, matchers};

use service::{AskRequest, DecisionsRequest, QuestionsRequest, ReviewRequest};

mod common;

/// A systemone mock that answers P(yes) = 0.97 for every noul question.
async fn app() -> (
    std::sync::Arc<service::Service>,
    String,
    MockServer,
    tempfile::TempDir,
) {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .keys()
                .map(|id| (id.clone(), json!({"type":"noul","noul":0.97})))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&mock)
        .await;
    let config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
        mock.uri(),
    ))
    .unwrap();
    let (service, _, directory) = common::service(config).await;
    let url = common::serve(service.clone(), None).await;

    (service, url, mock, directory)
}

async fn ask(service: &service::Service, request: Value) {
    let request: AskRequest = serde_json::from_value(request).unwrap();
    service.ask(request).await.unwrap();
}

#[tokio::test]
async fn questions_review_and_label_close_the_loop() {
    let (service, url, _mock, _dir) = app().await;
    for state in ["hi", "hello", "invoice attached"] {
        ask(&service, json!({"state":state,"spec":"greeting"})).await;
    }
    let urgent = json!({"id":"q","type":"noul","instructions":"Is this urgent?"});
    for state in ["server down", "lunch?"] {
        ask(&service, json!({"state":state,"questions":[urgent]})).await;
    }

    let questions = service
        .questions(QuestionsRequest::default())
        .await
        .unwrap()
        .questions;
    assert_eq!(questions.len(), 2);
    let spec = questions.iter().find(|row| row.spec.is_some()).unwrap();
    assert_eq!(
        (spec.decisions, spec.labelled, spec.labelable),
        (3, 0, true)
    );
    assert_eq!(spec.question.as_ref().unwrap()["instructions"], "Greeting?");
    // The ad-hoc text is stored, so a recurring question can be read and saved as a spec.
    let adhoc = questions.iter().find(|row| row.spec.is_none()).unwrap();
    assert_eq!((adhoc.decisions, adhoc.labelable), (2, false));
    assert_eq!(
        adhoc.question.as_ref().unwrap()["instructions"],
        "Is this urgent?"
    );
    assert!(adhoc.question.as_ref().unwrap().get("id").is_none());

    // Review hides the model's answer and lists only unlabelled decisions.
    let review = service
        .review(ReviewRequest {
            spec: "greeting".into(),
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(review.labels, ["yes", "no"]);
    assert_eq!(review.decisions.len(), 3);
    let item = serde_json::to_value(&review.decisions[0]).unwrap();
    assert_eq!(
        item.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["id", "state"]
    );
    let invoice = review
        .decisions
        .iter()
        .find(|item| item.state == "invoice attached")
        .unwrap();
    service.label(invoice.id, "no".into()).await.unwrap();
    let review = service
        .review(ReviewRequest {
            spec: "greeting".into(),
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(review.decisions.len(), 2);
    assert!(review.decisions.iter().all(|item| item.id != invoice.id));

    // The full list over REST shows the label and the answer by label name.
    let response: Value = reqwest::Client::new()
        .post(format!("{url}/v1/decisions"))
        .json(&json!({"spec":"greeting"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let labelled = response["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|decision| decision["id"] == invoice.id)
        .unwrap();
    assert_eq!(labelled["label"], "no");
    assert_eq!(labelled["probabilities"]["yes"], 0.97);
    assert!(response["labelable"].as_bool().unwrap());

    // An ad-hoc question is listed by its hash and cannot be labelled.
    let adhoc_decisions = service
        .decisions(DecisionsRequest {
            question: Some(adhoc.question_hash.clone()),
            ..DecisionsRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(adhoc_decisions.decisions.len(), 2);
    assert!(!adhoc_decisions.labelable);
    let questions = service
        .questions(QuestionsRequest::default())
        .await
        .unwrap()
        .questions;
    assert_eq!(
        questions
            .iter()
            .find(|row| row.spec.is_some())
            .unwrap()
            .labelled,
        1
    );
}

#[tokio::test]
async fn unknown_spec_missing_filter_and_zero_limit_are_rejected() {
    let (service, _url, _mock, _dir) = app().await;
    let missing = service
        .decisions(DecisionsRequest::default())
        .await
        .unwrap_err();
    assert!(
        missing.to_string().contains("give spec or question"),
        "{missing}"
    );
    let unknown = service
        .review(ReviewRequest {
            spec: "nope".into(),
            limit: None,
        })
        .await
        .unwrap_err();
    assert!(
        unknown.to_string().contains("unknown spec nope"),
        "{unknown}"
    );
    let zero = service
        .questions(QuestionsRequest { limit: Some(0) })
        .await
        .unwrap_err();
    assert!(
        zero.to_string().contains("limit must be at least 1"),
        "{zero}"
    );
}
