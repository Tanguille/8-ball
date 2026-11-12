//! Hosted providers: the decisions envelope, catalog detection, rate limits.

use std::time::{Duration, Instant};

use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use backend::Endpoint;
use domain::{config::BackendKind, question::Question};

fn config(url: &str, extra: &str) -> domain::config::EndpointConfig {
    toml::from_str(&format!("url='{url}'\n{extra}")).unwrap()
}
fn question() -> Question {
    serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Is this spam?"})).unwrap()
}
async fn decisions(server: &MockServer, template: ResponseTemplate) {
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/decisions"))
        .respond_with(template)
        .mount(server)
        .await;
}
/// POST requests only: detection also sends GET /props.
async fn posts(server: &MockServer) -> Vec<wiremock::Request> {
    let requests = server.received_requests().await.unwrap();
    requests
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect()
}
fn answer() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "result":{"model":"clef","answers":{"q":{"noul":0.9}}},"success":true,"errors":[],"messages":[]
    }))
}

#[tokio::test]
async fn decisions_sends_the_model_and_reads_the_result_envelope() {
    let server = MockServer::start().await;
    decisions(&server, answer()).await;
    let endpoint = Endpoint::new(
        "hosted",
        &config(&server.uri(), "kind='decisions'\nmodel='clef'"),
        None,
    );
    let probabilities = endpoint
        .ask("win a prize now", &[question()])
        .await
        .unwrap();
    assert!((probabilities[0][0] - 0.9).abs() < 1e-9);
    let body: serde_json::Value = serde_json::from_slice(&posts(&server).await[0].body).unwrap();
    assert_eq!(body["model"], "clef");
    assert_eq!(body["state"], "win a prize now");
    assert_eq!(body["questions"]["q"]["instructions"], "Is this spam?");
}

#[tokio::test]
async fn a_hosted_catalog_detects_decisions_and_chat_models() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            {"id":"clef","endpoints":["decisions"]},
            {"id":"qwen-3.8-27b","endpoints":["chat/completions","messages","responses"]}
        ]})))
        .mount(&server)
        .await;
    for (model, kind) in [
        ("clef", BackendKind::Decisions),
        ("qwen-3.8-27b", BackendKind::Logprob),
    ] {
        let endpoint = Endpoint::new(
            model,
            &config(&server.uri(), &format!("model='{model}'")),
            None,
        );
        assert_eq!(endpoint.info().await.unwrap().kind, kind);
    }
}

#[tokio::test]
async fn a_short_429_is_retried_after_its_retry_after() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    decisions(&server, answer()).await;
    let endpoint = Endpoint::new(
        "hosted",
        &config(&server.uri(), "kind='decisions'\nmodel='clef'"),
        None,
    );
    let start = Instant::now();
    endpoint.ask("hi", &[question()]).await.unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(posts(&server).await.len(), 2);
}

#[tokio::test]
async fn a_long_429_fails_at_once() {
    let server = MockServer::start().await;
    decisions(
        &server,
        ResponseTemplate::new(429).insert_header("Retry-After", "3600"),
    )
    .await;
    let endpoint = Endpoint::new(
        "hosted",
        &config(&server.uri(), "kind='decisions'\nmodel='clef'"),
        None,
    );
    let error = endpoint
        .ask("hi", &[question()])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("429"), "{error}");
    assert_eq!(posts(&server).await.len(), 1);
}

#[tokio::test]
async fn requests_per_minute_spaces_requests() {
    let server = MockServer::start().await;
    decisions(&server, answer()).await;
    // 600 per minute is one request per 100 ms.
    let endpoint = Endpoint::new(
        "hosted",
        &config(
            &server.uri(),
            "kind='decisions'\nmodel='clef'\nrequests_per_minute=600",
        ),
        None,
    );
    let start = Instant::now();
    for _ in 0..3 {
        endpoint.ask("hi", &[question()]).await.unwrap();
    }
    assert!(start.elapsed() >= Duration::from_millis(200));
}

#[tokio::test]
async fn a_second_429_is_not_retried_and_retry_after_0_waits_a_second() {
    let server = MockServer::start().await;
    decisions(
        &server,
        ResponseTemplate::new(429).insert_header("Retry-After", "0"),
    )
    .await;
    let endpoint = Endpoint::new(
        "hosted",
        &config(&server.uri(), "kind='decisions'\nmodel='clef'"),
        None,
    );
    let start = Instant::now();
    let error = endpoint
        .ask("hi", &[question()])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("429"), "{error}");
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(posts(&server).await.len(), 2);
}

#[tokio::test]
async fn a_502_is_retried_once() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    decisions(&server, answer()).await;
    let endpoint = Endpoint::new(
        "hosted",
        &config(&server.uri(), "kind='decisions'\nmodel='clef'"),
        None,
    );
    endpoint.ask("hi", &[question()]).await.unwrap();
    assert_eq!(posts(&server).await.len(), 2);
}
