use serde_json::{Value, json};

use domain::verdict;
use service::{AskRequest, Service};

#[path = "utils/common.rs"]
mod common;

async fn fixture(
    ad_hoc: bool,
) -> (
    std::sync::Arc<Service>,
    store::Store,
    wiremock::MockServer,
    tempfile::TempDir,
) {
    common::d3(&format!(
        "max_state_chars=10\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.greeting-copy]\ntype='noul'\ninstructions='Greeting?'\n[learning]\nad_hoc={ad_hoc}"
    ))
    .await
}
fn request(state: &str) -> Value {
    json!({"spec":"greeting","state":state})
}
#[tokio::test]
async fn cache_hashes_exactly_the_fitted_state() {
    let (service, _store, mock, _dir) = fixture(false).await;
    let first = common::ask(&service, request("abcde first middle z0123")).await;
    let second = common::ask(&service, request("abcde different middle z0123")).await;
    assert!(first.id > 0);
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(first.id, second.id);
    let requests = common::requests(&mock).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["state"], "abcde\n…\nz0123");
}
#[tokio::test]
async fn rejects_unknown_spec_and_mixed_question_sources() {
    let (service, _store, _mock, _dir) = fixture(false).await;
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
    spec_bands_and_ad_hoc_defaults(false).await;
}
#[tokio::test]
async fn ad_hoc_question_with_a_spec_text_keeps_its_own_calibration() {
    // The ad-hoc question shares the spec's stored text, so it is learnable, but its calibration key differs.
    spec_bands_and_ad_hoc_defaults(true).await;
}
async fn spec_bands_and_ad_hoc_defaults(learning_ad_hoc: bool) {
    let (service, store, mock, _dir) = fixture(learning_ad_hoc).await;
    let question = common::greeting();
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
        warnings: vec![],
    };
    store
        .put_calibration(
            &calibration_id,
            Some("greeting"),
            &verdict::Bands::Noul {
                yes_at: 1.01,
                no_at: -0.01,
            },
            &report,
        )
        .await
        .unwrap();
    let spec = common::ask(&service, request("hi")).await;
    assert!(spec.calibrated);
    assert_eq!(spec.verdict, "unsure");
    let adhoc = common::ask(&service, json!({"state":"hi","questions":[question]})).await;
    assert!(!adhoc.calibrated);
    assert!(!adhoc.cached);
    assert_eq!(adhoc.verdict, "yes");
}

#[tokio::test]
async fn cache_ids_and_labels_stay_in_the_requested_scope() {
    let (service, store, _mock, _dir) = fixture(false).await;
    let adhoc = common::ask(
        &service,
        json!({"state":"hello","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}),
    )
    .await;
    let named = common::ask(&service, request("hello")).await;
    assert!(!named.cached);
    assert_ne!(adhoc.id, named.id);
    service.label(named.id, "yes".into()).await.unwrap();
    let copy_request = json!({"spec":"greeting-copy","state":"hello"});
    let copy = common::ask(&service, copy_request.clone()).await;
    assert!(!copy.cached);
    assert_ne!(named.id, copy.id);
    service.label(copy.id, "no".into()).await.unwrap();
    let again = common::ask(&service, copy_request).await;
    assert!(again.cached);
    assert_eq!(again.id, copy.id);
    let question_hash = verdict::question_hash(&common::greeting());
    assert_eq!(
        store
            .labelled(Some("greeting"), &question_hash)
            .await
            .unwrap()[0]
            .label,
        "yes"
    );
    assert_eq!(
        store
            .labelled(Some("greeting-copy"), &question_hash)
            .await
            .unwrap()[0]
            .label,
        "no"
    );
    assert!(service.label(adhoc.id, "yes".into()).await.is_err());
}
