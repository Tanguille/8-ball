use eightball::{backend::Endpoint, config::Config, http::router, service::Service};
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn app(
    status: u16,
) -> (
    String,
    MockServer,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":0.97}}})),
        )
        .mount(&mock)
        .await;
    let cfg: Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n",
        mock.uri()
    ))
    .unwrap();
    let endpoints: IndexMap<_, _> = [(
        "d3".into(),
        Endpoint::new("d3", &cfg.endpoints["d3"], None).unwrap(),
    )]
    .into();
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = eightball::store::Store::open(&dir.path().join("db.sqlite")).unwrap();
    let service = Arc::new(Service::new(cfg, endpoints, store));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router(service, None, &tokio_util::sync::CancellationToken::new()),
        )
        .await
        .unwrap();
    });
    (url, mock, task, dir)
}
#[tokio::test]
async fn noul_ask_returns_probabilities_and_verdict() {
    let (url, _mock, task, _dir) = app(200).await;
    let r=reqwest::Client::new().post(format!("{url}/v1/ask")).json(&json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]})).send().await.unwrap();
    assert!(r.status().is_success());
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["results"][0]["verdict"], "yes");
    assert_eq!(body["results"][0]["probabilities"]["yes"], 0.97);
    assert!(body["results"][0]["probabilities"]["no"].as_f64().unwrap() > 0.02);
    assert!(body["results"][0]["id"].as_i64().unwrap() > 0);
    assert_eq!(body["results"][0]["cached"], false);
    assert_eq!(body["results"][0]["calibrated"], false);
    task.abort();
}
#[tokio::test]
async fn backend_failure_is_502() {
    let (url, _mock, task, _dir) = app(500).await;
    let r=reqwest::Client::new().post(format!("{url}/v1/ask")).json(&json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]})).send().await.unwrap();
    assert_eq!(r.status(), 502);
    assert!(
        r.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("500")
    );
    task.abort();
}
#[tokio::test]
async fn empty_questions_is_400() {
    let (url, _mock, task, _dir) = app(200).await;
    let r = reqwest::Client::new()
        .post(format!("{url}/v1/ask"))
        .json(&json!({"state":"hi","questions":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    task.abort();
}
#[tokio::test]
async fn healthz_is_ok() {
    let (url, _mock, task, _dir) = app(200).await;
    let r = reqwest::get(format!("{url}/healthz")).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");
    task.abort();
}
