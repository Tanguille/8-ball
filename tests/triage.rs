use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

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
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|response: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&response.body).unwrap();
            let probabilities = match body["state"].as_str().unwrap() {
                "bad" => return ResponseTemplate::new(500),
                "no" => 0.01,
                "unsure" => 0.5,
                _ => 0.97,
            };
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .keys()
                .map(|id| (id.clone(), json!({"type":"noul","noul":probabilities})))
                .collect();

            ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&model)
        .await;
    if let Some(healthy) = embed {
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/embeddings"))
            .respond_with(move |request: &wiremock::Request| {
                if !healthy {
                    return ResponseTemplate::new(500);
                }
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let data: Vec<_> = body["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(index, input)| {
                        let vector =
                            if index == 0 || matches!(input.as_str(), Some("three" | "four")) {
                                [1.0, 0.0]
                            } else {
                                [0.0, 1.0]
                            };

                        json!({"index":index,"embedding":vector})
                    })
                    .collect();

                ResponseTemplate::new(200).set_body_json(json!({"data":data}))
            })
            .mount(&embedding)
            .await;
    }
    let extra = if embed.is_some() {
        format!("\n[embedder]\nurl='{}'\nmodel='embed'", embedding.uri())
    } else {
        String::new()
    };
    let config: eightball::config::Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n{extra}\n[specs.relevant]\ntype='noul'\ninstructions='Is this relevant?'",
        model.uri()
    ))
    .unwrap();
    let endpoint = eightball::backend::Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = eightball::store::Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let service = std::sync::Arc::new(eightball::service::Service::new(
        config,
        [("d3".into(), endpoint)].into(),
        store,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            eightball::http::router(service, None, &tokio_util::sync::CancellationToken::new()),
        )
        .await
        .unwrap();
    });

    (url, model, embedding, task, directory)
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
    let response = triage(
        &url,
        &request(&json!(["yes1", "yes2", "unsure", "no", "yes1"])),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
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
            .filter(|response| response.method == "POST")
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
async fn invalid_keep_is_rejected_before_model_requests() {
    let (url, model, _embedding, task, _dir) = app(None).await;
    for (question, keep) in [
        (
            json!({"id":"q","type":"noul","instructions":"Relevant?"}),
            "typo",
        ),
        (
            json!({"id":"q","type":"choice","instructions":"Which?","criteria":{"a":"","b":""}}),
            "yes",
        ),
        (
            json!({"id":"q","type":"score","instructions":"Urgent?","criteria":["low","high"]}),
            "no",
        ),
        (
            json!({"id":"q","type":"score","instructions":"Urgent?","criteria":["low","high"]}),
            "2",
        ),
        (
            json!({"id":"q","type":"score","instructions":"Urgent?","criteria":["low","high"]}),
            "-1",
        ),
    ] {
        let response = triage(
            &url,
            &json!({"items":["one"],"question":question,"keep":keep}),
        )
        .await;
        assert_eq!(response.status(), 400, "keep={keep}");
        assert!(response.text().await.unwrap().contains("keep"));
    }
    assert!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method != "POST")
    );
    task.abort();
}
#[tokio::test]
async fn prefilter_judges_only_closest_items() {
    let (url, model, _embedding, task, _dir) = app(Some(true)).await;
    let mut body = request(&json!(["one", "two", "three", "four"]));
    body["top_k"] = json!(2);
    let response = triage(&url, &body).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["kept"][0]["index"], 2);
    assert_eq!(body["kept"][1]["index"], 3);
    assert_eq!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|response| response.method == "POST")
            .count(),
        2
    );
    task.abort();
}
#[tokio::test]
async fn named_spec_triage_uses_the_spec_question_id() {
    let (url, _model, _embedding, task, _dir) = app(None).await;
    let response = triage(&url, &json!({"items":["yes1","no"],"spec":"relevant"})).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["kept"][0]["decision"]["question"], "relevant");
    assert_eq!(body["dropped"], 1);
    task.abort();
}
#[tokio::test]
async fn prefilter_handles_a_smaller_batch() {
    let (url, model, _embedding, task, _dir) = app(Some(true)).await;
    let mut body = request(&json!(["one", "three"]));
    body["top_k"] = json!(1);
    let response = triage(&url, &body).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["kept"][0]["index"], 1);
    assert!(body.get("warning").is_none());
    assert_eq!(
        model
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    task.abort();
}
#[tokio::test]
async fn failed_embedder_judges_every_item_with_warning() {
    let (url, model, _embedding, task, _dir) = app(Some(false)).await;
    let mut body = request(&json!(["one", "two", "three", "four"]));
    body["top_k"] = json!(2);
    let response = triage(&url, &body).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
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
            .filter(|response| response.method == "POST")
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
