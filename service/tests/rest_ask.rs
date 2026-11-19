use reqwest::Client;
use serde_json::{Value, json};

mod common;

async fn app() -> (String, wiremock::MockServer, tempfile::TempDir) {
    let mock = common::systemone().await;
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
    let (url, _mock, _dir) = app().await;
    let response = Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["results"][0]["verdict"], "yes");
    assert_eq!(body["results"][0]["probabilities"]["yes"], 0.97);
    assert!((body["results"][0]["probabilities"]["no"].as_f64().unwrap() - 0.03).abs() < 1e-9);
    assert!(body["results"][0]["id"].as_i64().unwrap() > 0);
    assert_eq!(body["results"][0]["cached"], false);
    assert_eq!(body["results"][0]["calibrated"], false);
}
#[tokio::test]
async fn backend_failure_is_502() {
    let (url, _mock, _dir) = app().await;
    let response = Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"fail","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}))
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
    let (url, _mock, _dir) = app().await;
    let response = Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}
#[tokio::test]
async fn healthz_is_ok() {
    let (url, _mock, _dir) = app().await;
    let response = reqwest::get(format!("{url}/healthz")).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "ok");
}
