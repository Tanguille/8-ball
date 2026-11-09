use serde_json::{Map, Value, json};
use wiremock::{MockServer, ResponseTemplate, matchers};

use service::AskRequest;

mod common;

async fn app() -> (
    String,
    std::sync::Arc<service::Service>,
    MockServer,
    tempfile::TempDir,
) {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if body["state"] == "fail" {
                return ResponseTemplate::new(500);
            }
            let answers: Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = if question["type"] == "choice" {
                        let names = question["criteria"].as_object().unwrap();
                        let probabilities: Map<String, Value> = names
                            .keys()
                            .enumerate()
                            .map(|(i, name)| {
                                (name.clone(), json!(if i == 0 { 0.99 } else { 0.01 }))
                            })
                            .collect();
                        json!({"type":"choice","probabilities":probabilities})
                    } else {
                        json!({"type":"noul","noul":0.97})
                    };

                    (id.clone(), answer)
                })
                .collect();

            ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&mock)
        .await;
    let config: domain::config::Config =
        toml::from_str(
            &format!(
                "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
                mock.uri(),
            ),
        )
        .unwrap();
    let (service, _, directory) = common::service(config).await;
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
