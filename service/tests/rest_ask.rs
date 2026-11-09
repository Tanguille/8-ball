use serde_json::{Value, json};
use wiremock::{MockServer, matchers};

mod common;

async fn app(status: u16) -> (String, MockServer, tempfile::TempDir) {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(
            wiremock::ResponseTemplate::new(status)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":0.97}}})),
        )
        .mount(&mock)
        .await;
    let endpoint_config: domain::config::Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n",
        mock.uri()
    ))
    .unwrap();
    let (service, _, directory) = common::service(endpoint_config).await;

    (common::serve(service, None).await, mock, directory)
}
#[tokio::test]
async fn noul_ask_returns_probabilities_and_verdict() {
    let (url, _mock, _dir) = app(200).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["results"][0]["verdict"], "yes");
    assert_eq!(body["results"][0]["probabilities"]["yes"], 0.97);
    assert!(body["results"][0]["probabilities"]["no"].as_f64().unwrap() > 0.02);
    assert!(body["results"][0]["id"].as_i64().unwrap() > 0);
    assert_eq!(body["results"][0]["cached"], false);
    assert_eq!(body["results"][0]["calibrated"], false);
}
#[tokio::test]
async fn backend_failure_is_502() {
    let (url, _mock, _dir) = app(500).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    assert!(
        response.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("500")
    );
}
#[tokio::test]
async fn empty_questions_is_400() {
    let (url, _mock, _dir) = app(200).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}
#[tokio::test]
async fn healthz_is_ok() {
    let (url, _mock, _dir) = app(200).await;
    let response = reqwest::get(format!("{url}/healthz")).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "ok");
}
