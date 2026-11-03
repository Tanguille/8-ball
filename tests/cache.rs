use eightball::{
    backend::Endpoint,
    config::{BackendKind, Config},
    question::Question,
    service::{AskRequest, Service},
    store::Store,
    verdict::{Bands, Report, calibration_id, question_hash},
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

async fn fixture() -> (Service, Store, MockServer, tempfile::TempDir) {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|r: &Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
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
    let config: Config=toml::from_str(&format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\nmax_state_chars=10\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.greeting-copy]\ntype='noul'\ninstructions='Greeting?'",mock.uri())).unwrap();
    let ep = Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.sqlite")).await.unwrap();
    (
        Service::new(config, [("d3".into(), ep)].into(), store.clone()),
        store,
        mock,
        dir,
    )
}
fn request(state: &str) -> AskRequest {
    serde_json::from_value(json!({"spec":"greeting","state":state})).unwrap()
}
#[tokio::test]
async fn repeated_ask_reuses_answer_and_row_id() {
    let (service, _store, mock, _dir) = fixture().await;
    let first = service
        .ask(request("hello"))
        .await
        .unwrap()
        .results
        .remove(0);
    let second = service
        .ask(request("hello"))
        .await
        .unwrap()
        .results
        .remove(0);
    assert!(first.id > 0);
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(first.id, second.id);
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}
#[tokio::test]
async fn cache_hashes_exactly_the_fitted_state() {
    let (service, _store, mock, _dir) = fixture().await;
    let first = service
        .ask(request("abcde first middle z0123"))
        .await
        .unwrap()
        .results
        .remove(0);
    let second = service
        .ask(request("abcde different middle z0123"))
        .await
        .unwrap()
        .results
        .remove(0);
    assert!(second.cached);
    assert_eq!(first.id, second.id);
    let requests = mock.received_requests().await.unwrap();
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(body["state"], "abcde\n…\nz0123");
}
#[tokio::test]
async fn rejects_unknown_spec_and_mixed_question_sources() {
    let (service, _store, _mock, _dir) = fixture().await;
    let req: AskRequest = serde_json::from_value(json!({"spec":"unknown","state":"hi"})).unwrap();
    assert!(
        service
            .ask(req)
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown spec unknown")
    );
    let req: AskRequest=serde_json::from_value(json!({"spec":"greeting","state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]})).unwrap();
    assert!(
        service
            .ask(req)
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
    let cid = calibration_id(
        &question_hash(&question),
        BackendKind::Systemone,
        &mock.uri(),
    );
    let report = Report {
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
            &cid,
            "greeting",
            &Bands::Noul {
                yes_at: 1.01,
                no_at: -0.01,
            },
            &report,
        )
        .await
        .unwrap();
    let spec = service.ask(request("hi")).await.unwrap().results.remove(0);
    assert!(spec.calibrated);
    assert_eq!(spec.verdict, "unsure");
    let adhoc = service
        .ask(AskRequest {
            state: "hi".into(),
            spec: None,
            questions: vec![question],
            endpoint: None,
        })
        .await
        .unwrap()
        .results
        .remove(0);
    assert!(!adhoc.calibrated);
    assert!(!adhoc.cached);
    assert_eq!(adhoc.verdict, "yes");
}

#[tokio::test]
async fn cache_ids_and_labels_stay_in_the_requested_scope() {
    let (service, store, _mock, _dir) = fixture().await;
    let adhoc: AskRequest = serde_json::from_value(json!({
        "state":"hello", "questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]
    }))
    .unwrap();
    let ad = service.ask(adhoc).await.unwrap().results.remove(0);
    let named = service
        .ask(request("hello"))
        .await
        .unwrap()
        .results
        .remove(0);
    assert!(!named.cached);
    assert_ne!(ad.id, named.id);
    service.label(named.id, "yes".into()).await.unwrap();
    let copy_request: AskRequest =
        serde_json::from_value(json!({"spec":"greeting-copy","state":"hello"})).unwrap();
    let copy = service
        .ask(copy_request.clone())
        .await
        .unwrap()
        .results
        .remove(0);
    assert!(!copy.cached);
    assert_ne!(named.id, copy.id);
    service.label(copy.id, "no".into()).await.unwrap();
    let again = service.ask(copy_request).await.unwrap().results.remove(0);
    assert!(again.cached);
    assert_eq!(again.id, copy.id);
    let q: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Greeting?"})).unwrap();
    let qh = question_hash(&q);
    assert_eq!(
        store.labelled("greeting", &qh).await.unwrap()[0].label,
        "yes"
    );
    assert_eq!(
        store.labelled("greeting-copy", &qh).await.unwrap()[0].label,
        "no"
    );
    assert!(service.label(ad.id, "yes".into()).await.is_err());
}
