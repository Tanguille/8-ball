use eightball::{backend::Endpoint, config::Config, http::router, service::Service};
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc};
use tokio::io::AsyncWriteExt;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

async fn app(
    embed: Option<bool>,
) -> (
    String,
    MockServer,
    MockServer,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let model = MockServer::start().await;
    let embedding = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|r: &Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let p = match body["state"].as_str().unwrap() {
                "bad" => return ResponseTemplate::new(500),
                "no" => 0.01,
                "unsure" => 0.5,
                _ => 0.97,
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":p}}}))
        })
        .mount(&model)
        .await;
    if let Some(healthy) = embed {
        Mock::given(method("POST")).and(path("/v1/embeddings")).respond_with(if healthy {
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"index":0,"embedding":[1.0,0.0]},{"index":1,"embedding":[0.0,1.0]},{"index":2,"embedding":[0.0,1.0]},{"index":3,"embedding":[1.0,0.0]},{"index":4,"embedding":[1.0,0.0]}]}))
        } else {ResponseTemplate::new(500)}).mount(&embedding).await;
    }
    let extra = if embed.is_some() {
        format!("\n[embedder]\nurl='{}'\nmodel='embed'", embedding.uri())
    } else {
        String::new()
    };
    let config: Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n{extra}",
        model.uri()
    ))
    .unwrap();
    let ep = Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = eightball::store::Store::open(&dir.path().join("db.sqlite"))
        .await
        .unwrap();
    let service = Arc::new(Service::new(config, [("d3".into(), ep)].into(), store));
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
    (url, model, embedding, task, dir)
}
fn request(items: &Value) -> Value {
    json!({"items":items,"question":{"id":"q","type":"noul","instructions":"Is this relevant?"}})
}
async fn triage(url: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{url}/v1/triage"))
        .json(body)
        .send()
        .await
        .unwrap()
}
#[tokio::test]
async fn dedupes_and_partitions_verdicts() {
    let (url, model, _embedding, task, _dir) = app(None).await;
    let r = triage(
        &url,
        &request(&json!(["yes1", "yes2", "unsure", "no", "yes1"])),
    )
    .await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["kept"].as_array().unwrap().len(), 2);
    assert_eq!(body["unsure"].as_array().unwrap().len(), 1);
    assert_eq!(body["dropped"], 1);
    assert_eq!(body["kept"][0]["index"], 0);
    assert_eq!(body["kept"][1]["index"], 1);
    assert_eq!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        4
    );
    task.abort();
}
#[tokio::test]
async fn sixty_five_items_rejected() {
    let (url, _model, _embedding, task, _dir) = app(None).await;
    assert_eq!(
        triage(&url, &request(&json!(vec!["item"; 65])))
            .await
            .status(),
        400
    );
    task.abort();
}
#[tokio::test]
async fn prefilter_judges_only_closest_items() {
    let (url, model, _embedding, task, _dir) = app(Some(true)).await;
    let mut body = request(&json!(["one", "two", "three", "four"]));
    body["top_k"] = json!(2);
    let r = triage(&url, &body).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["kept"][0]["index"], 2);
    assert_eq!(body["kept"][1]["index"], 3);
    assert_eq!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        2
    );
    task.abort();
}
#[tokio::test]
async fn failed_embedder_judges_every_item_with_warning() {
    let (url, model, _embedding, task, _dir) = app(Some(false)).await;
    let mut body = request(&json!(["one", "two", "three", "four"]));
    body["top_k"] = json!(2);
    let r = triage(&url, &body).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["kept"].as_array().unwrap().len(), 4);
    assert!(
        body["warning"]
            .as_str()
            .unwrap()
            .starts_with("prefilter skipped")
    );
    assert_eq!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        4
    );
    task.abort();
}
#[tokio::test]
async fn top_k_requires_embedder() {
    let (url, _model, _embedding, task, _dir) = app(None).await;
    let mut body = request(&json!(["one", "two"]));
    body["top_k"] = json!(1);
    assert_eq!(triage(&url, &body).await.status(), 400);
    task.abort();
}
#[tokio::test]
async fn one_failed_judgment_fails_whole_triage() {
    let (url, _model, _embedding, task, _dir) = app(None).await;
    assert_eq!(
        triage(&url, &request(&json!(["yes1", "bad"])))
            .await
            .status(),
        502
    );
    task.abort();
}
#[tokio::test]
async fn cli_reads_json_items_from_stdin() {
    let (url, _model, _embedding, task, _dir) = app(None).await;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["triage", "Is this relevant?", "--url", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"["yes1","no"]"#)
        .await
        .unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["kept"][0]["index"], 0);
    assert_eq!(body["dropped"], 1);
    task.abort();
}
