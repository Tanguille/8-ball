use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::{MockServer, ResponseTemplate, matchers};

use eightball::service::{AskRequest, Service};

async fn app() -> (
    String,
    Arc<Service>,
    MockServer,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|response: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&response.body).unwrap();
            if body["state"] == "fail" {
                return ResponseTemplate::new(500);
            }
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = if question["type"] == "choice" {
                        let names = question["criteria"].as_object().unwrap();
                        let probabilities: serde_json::Map<String, Value> = names
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
    let config: eightball::config::Config =
        toml::from_str(
            &format!(
                "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
                mock.uri(),
            ),
        )
        .unwrap();
    let endpoint = eightball::backend::Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = eightball::store::Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let service = Arc::new(Service::new(
        config,
        [("d3".into(), endpoint)].into(),
        store,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = service.clone();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            eightball::http::router(served, None, &tokio_util::sync::CancellationToken::new()),
        )
        .await
        .unwrap();
    });

    (url, service, mock, directory, task)
}
#[tokio::test]
async fn ad_hoc_decision_has_bounded_labels_and_openmetrics_type() {
    let (url, service, _mock, _dir, task) = app().await;
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
        text.contains("eightball_decisions_total{endpoint=\"d3\",spec=\"\",verdict=\"decided\"} 1"),
        "{text}"
    );
    assert!(!text.contains("private-client-label"));
    assert!(!text.contains("another-client-label"));
    task.abort();
}
#[tokio::test]
async fn backend_errors_are_counted() {
    let (url, service, _mock, _dir, task) = app().await;
    let request: AskRequest = serde_json::from_value(
        json!({"state":"fail","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}),
    )
    .unwrap();
    assert!(service.ask(request).await.is_err());
    let text = reqwest::get(format!("{url}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        text.contains("eightball_backend_errors_total{endpoint=\"d3\"} 1"),
        "{text}"
    );
    task.abort();
}
#[tokio::test]
async fn spec_verdict_and_cache_hits_are_counted() {
    let (url, service, _mock, _dir, task) = app().await;
    let request: AskRequest =
        serde_json::from_value(json!({"state":"hi","spec":"greeting"})).unwrap();
    service.ask(request.clone()).await.unwrap();
    assert!(service.ask(request).await.unwrap().results[0].cached);
    let text = reqwest::get(format!("{url}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        text.contains(
            "eightball_decisions_total{endpoint=\"d3\",spec=\"greeting\",verdict=\"yes\"} 2"
        ),
        "{text}"
    );
    task.abort();
}
