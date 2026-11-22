use serde_json::json;

use service::AskRequest;

mod common;

async fn app() -> (
    String,
    std::sync::Arc<service::Service>,
    wiremock::MockServer,
    tempfile::TempDir,
) {
    let (service, _, mock, directory) =
        common::d3("[specs.greeting]\ntype='noul'\ninstructions='Greeting?'").await;
    let url = common::serve(service.clone(), None).await;

    (url, service, mock, directory)
}
#[tokio::test]
async fn ad_hoc_decision_has_bounded_labels_and_openmetrics_type() {
    let (url, service, _mock, _dir) = app().await;
    let request: AskRequest = serde_json::from_value(json!({"state":"hi","questions":[{"id":"q","type":"choice","instructions":"Pick","criteria":{"private-client-label":"","another-client-label":""}}]}))
    .unwrap();
    service.ask(request).await.unwrap();
    let response = reqwest::get(format!("{url}/metrics")).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/openmetrics-text; version=1.0.0; charset=utf-8"
    );
    let text = response.text().await.unwrap();
    assert!(
        text.contains(
            "eightball_decisions_total{endpoint=\"d3\",spec=\"\",verdict=\"decided\",cached=\"false\"} 1"
        ),
        "{text}"
    );
    assert!(!text.contains("private-client-label"));
    assert!(!text.contains("another-client-label"));
}
#[tokio::test]
async fn backend_errors_are_counted() {
    let (_url, service, _mock, _dir) = app().await;
    let request: AskRequest = serde_json::from_value(
        json!({"state":"fail","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}),
    )
    .unwrap();
    assert!(service.ask(request).await.is_err());
    let text = service.metrics();
    assert!(
        text.contains("eightball_backend_errors_total{endpoint=\"d3\"} 1"),
        "{text}"
    );
    // Failed calls are timed too.
    assert!(
        text.contains("eightball_backend_request_duration_seconds_count{endpoint=\"d3\"} 1"),
        "{text}"
    );
    // Slow hosted calls need finite buckets above the default endpoint timeout.
    for ceiling in ["25.6", "51.2", "102.4"] {
        assert!(
            text.contains(&format!(
                "eightball_backend_request_duration_seconds_bucket{{le=\"{ceiling}\",endpoint=\"d3\"}} 1"
            )),
            "{text}"
        );
    }
}
#[tokio::test]
async fn trimmed_states_are_counted() {
    let (_url, service, _mock, _dir) = app().await;
    let request: AskRequest =
        serde_json::from_value(json!({"state":"x".repeat(2001),"spec":"greeting"})).unwrap();
    service.ask(request).await.unwrap();
    let text = service.metrics();
    assert!(
        text.contains("eightball_trimmed_states_total{endpoint=\"d3\"} 1"),
        "{text}"
    );
}
#[tokio::test]
async fn spec_verdict_and_cache_hits_are_counted() {
    let (_url, service, _mock, _dir) = app().await;
    let request: AskRequest =
        serde_json::from_value(json!({"state":"hi","spec":"greeting"})).unwrap();
    service.ask(request.clone()).await.unwrap();
    assert!(service.ask(request).await.unwrap().results[0].cached);
    let text = service.metrics();
    for cached in ["false", "true"] {
        assert!(
            text.contains(&format!(
                "eightball_decisions_total{{endpoint=\"d3\",spec=\"greeting\",verdict=\"yes\",cached=\"{cached}\"}} 1"
            )),
            "{text}"
        );
    }
    // The cache hit never reached the backend.
    assert!(
        text.contains("eightball_backend_request_duration_seconds_count{endpoint=\"d3\"} 1"),
        "{text}"
    );
}
