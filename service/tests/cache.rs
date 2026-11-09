use serde_json::{Value, json};
use wiremock::{MockServer, matchers};

use domain::{question::Question, verdict};
use service::{AskRequest, Service};
use store::Store;

async fn fixture() -> (Service, Store, MockServer, tempfile::TempDir) {
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

            wiremock::ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&mock)
        .await;
    let config: domain::config::Config =
        toml::from_str(
            &format!(
                "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\nmax_state_chars=10\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.greeting-copy]\ntype='noul'\ninstructions='Greeting?'",
                mock.uri(),
            ),
        )
        .unwrap();
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();

    (Service::new(config, store.clone()), store, mock, directory)
}
fn request(state: &str) -> AskRequest {
    serde_json::from_value(json!({"spec":"greeting","state":state})).unwrap()
}
async fn ask_one(service: &Service, request: AskRequest) -> service::Decision {
    service.ask(request).await.unwrap().results.remove(0)
}
#[tokio::test]
async fn cache_hashes_exactly_the_fitted_state() {
    let (service, _store, mock, _dir) = fixture().await;
    let first = ask_one(&service, request("abcde first middle z0123")).await;
    let second = ask_one(&service, request("abcde different middle z0123")).await;
    assert!(first.id > 0);
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(first.id, second.id);
    let requests = mock.received_requests().await.unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(body["state"], "abcde\n…\nz0123");
}
#[tokio::test]
async fn rejects_unknown_spec_and_mixed_question_sources() {
    let (service, _store, _mock, _dir) = fixture().await;
    let request: AskRequest =
        serde_json::from_value(json!({"spec":"unknown","state":"hi"})).unwrap();
    assert!(
        service
            .ask(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown spec unknown")
    );
    let request: AskRequest = serde_json::from_value(json!({"spec":"greeting","state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}))
    .unwrap();
    assert!(
        service
            .ask(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("use spec or questions, not both")
    );
}
#[tokio::test]
async fn spec_uses_bands_but_ad_hoc_uses_defaults() {
    let (service, store, mock, _dir) = fixture().await;
    let question: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Greeting?"})).unwrap();
    let calibration_id = verdict::calibration_id(
        &verdict::question_hash(&question),
        domain::config::BackendKind::Systemone,
        &mock.uri(),
    );
    let report = verdict::Report {
        examples: 40,
        decided: 0,
        correct: 0,
        precision: 0.0,
        coverage: 0.0,
        target: 0.95,
        in_sample: true,
        warnings: vec![],
    };
    store
        .put_calibration(
            &calibration_id,
            "greeting",
            &verdict::Bands::Noul {
                yes_at: 1.01,
                no_at: -0.01,
            },
            &report,
        )
        .await
        .unwrap();
    let spec = ask_one(&service, request("hi")).await;
    assert!(spec.calibrated);
    assert_eq!(spec.verdict, "unsure");
    let adhoc = ask_one(
        &service,
        AskRequest {
            state: "hi".into(),
            spec: None,
            questions: vec![question],
            endpoint: None,
        },
    )
    .await;
    assert!(!adhoc.calibrated);
    assert!(!adhoc.cached);
    assert_eq!(adhoc.verdict, "yes");
}

#[tokio::test]
async fn cache_ids_and_labels_stay_in_the_requested_scope() {
    let (service, store, _mock, _dir) = fixture().await;
    let adhoc_request: AskRequest = serde_json::from_value(json!({
        "state":"hello", "questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]
    }))
    .unwrap();
    let adhoc = ask_one(&service, adhoc_request).await;
    let named = ask_one(&service, request("hello")).await;
    assert!(!named.cached);
    assert_ne!(adhoc.id, named.id);
    service.label(named.id, "yes".into()).await.unwrap();
    let copy_request: AskRequest =
        serde_json::from_value(json!({"spec":"greeting-copy","state":"hello"})).unwrap();
    let copy = ask_one(&service, copy_request.clone()).await;
    assert!(!copy.cached);
    assert_ne!(named.id, copy.id);
    service.label(copy.id, "no".into()).await.unwrap();
    let again = ask_one(&service, copy_request).await;
    assert!(again.cached);
    assert_eq!(again.id, copy.id);
    let question: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Greeting?"})).unwrap();
    let question_hash = verdict::question_hash(&question);
    assert_eq!(
        store.labelled("greeting", &question_hash).await.unwrap()[0].label,
        "yes"
    );
    assert_eq!(
        store
            .labelled("greeting-copy", &question_hash)
            .await
            .unwrap()[0]
            .label,
        "no"
    );
    assert!(service.label(adhoc.id, "yes".into()).await.is_err());
}
