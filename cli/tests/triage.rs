use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

mod common;

async fn app(embed: Option<bool>) -> (String, MockServer, MockServer, tempfile::TempDir) {
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
    let config: domain::config::Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n{extra}\n[specs.relevant]\ntype='noul'\ninstructions='Is this relevant?'",
        model.uri()
    ))
    .unwrap();
    let (service, _, directory) = common::service(config).await;
    let url = common::serve(service, None).await;

    (url, model, embedding, directory)
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
async fn posts(model: &MockServer) -> usize {
    model
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
        .count()
}
async fn cli_triage(args: &[&str], items: &[u8]) -> std::process::Output {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .arg("triage")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(items).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    output
}
#[tokio::test]
async fn dedupes_and_partitions_verdicts() {
    let (url, model, _embedding, _dir) = app(None).await;
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
    assert_eq!(posts(&model).await, 4);
}
#[tokio::test]
async fn sixty_five_items_rejected() {
    let (url, _model, _embedding, _dir) = app(None).await;
    assert_eq!(
        triage(&url, &request(&json!(vec!["item"; 65])))
            .await
            .status(),
        400
    );
}
#[tokio::test]
async fn invalid_keep_is_rejected_before_model_requests() {
    let (url, model, _embedding, _dir) = app(None).await;
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
    assert_eq!(posts(&model).await, 0);
}
#[tokio::test]
async fn prefilter_judges_only_closest_items() {
    let (url, model, _embedding, _dir) = app(Some(true)).await;
    let mut body = request(&json!(["one", "two", "three", "four"]));
    body["top_k"] = json!(2);
    let response = triage(&url, &body).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["kept"][0]["index"], 2);
    assert_eq!(body["kept"][1]["index"], 3);
    assert!(body.get("warning").is_none());
    assert_eq!(posts(&model).await, 2);
}
#[tokio::test]
async fn named_spec_triage_uses_the_spec_question_id() {
    let (url, _model, _embedding, _dir) = app(None).await;
    let response = triage(&url, &json!({"items":["yes1","no"],"spec":"relevant"})).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["kept"][0]["decision"]["question"], "relevant");
    assert_eq!(body["dropped"], 1);
}
#[tokio::test]
async fn failed_embedder_judges_every_item_with_warning() {
    let (url, model, _embedding, _dir) = app(Some(false)).await;
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
    assert_eq!(posts(&model).await, 4);
}
#[tokio::test]
async fn top_k_requires_embedder() {
    let (url, _model, _embedding, _dir) = app(None).await;
    let mut body = request(&json!(["one", "two"]));
    body["top_k"] = json!(1);
    assert_eq!(triage(&url, &body).await.status(), 400);
}
#[tokio::test]
async fn one_failed_judgment_fails_whole_triage() {
    let (url, _model, _embedding, _dir) = app(None).await;
    assert_eq!(
        triage(&url, &request(&json!(["yes1", "bad"])))
            .await
            .status(),
        502
    );
}
#[tokio::test]
async fn cli_reads_json_items_from_stdin() {
    let (url, _model, _embedding, _dir) = app(None).await;
    let output = cli_triage(&["Is this relevant?", "--url", &url], br#"["yes1","no"]"#).await;
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["kept"][0]["index"], 0);
    assert_eq!(body["dropped"], 1);
}

#[tokio::test]
async fn context_is_model_data_and_part_of_cache_identity() {
    let (url, model, _embedding, _dir) = app(None).await;
    let mut body =
        json!({"items":[{"title":"backup change"}],"spec":"relevant","context":"uses kopiur"});
    let first: Value = triage(&url, &body).await.json().await.unwrap();
    let repeated: Value = triage(&url, &body).await.json().await.unwrap();
    body["context"] = json!("uses another backup tool");
    let changed: Value = triage(&url, &body).await.json().await.unwrap();
    body["context"] = json!("uses kopiur");
    body["items"] = json!([{"title":"other change"}]);
    let changed_item: Value = triage(&url, &body).await.json().await.unwrap();
    assert_eq!(first["kept"][0]["decision"]["cached"], false);
    assert_eq!(repeated["kept"][0]["decision"]["cached"], true);
    assert_eq!(changed["kept"][0]["decision"]["cached"], false);
    assert_eq!(changed_item["kept"][0]["decision"]["cached"], false);
    assert_ne!(
        first["kept"][0]["decision"]["id"],
        changed_item["kept"][0]["decision"]["id"]
    );
    assert_ne!(
        first["kept"][0]["decision"]["id"],
        changed["kept"][0]["decision"]["id"]
    );
    let requests = model.received_requests().await.unwrap();
    let calls: Vec<Value> = requests
        .iter()
        .filter(|request| request.method == "POST")
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    assert_eq!(calls.len(), 3);
    let state: Value = serde_json::from_str(calls[0]["state"].as_str().unwrap()).unwrap();
    assert_eq!(
        state,
        json!({"context":"uses kopiur","item":{"title":"backup change"}})
    );
    assert_eq!(calls[0]["questions"], calls[1]["questions"]);
}

#[tokio::test]
async fn oversized_contextual_batch_is_rejected_before_any_judgment() {
    let (url, model, embedding, _dir) = app(Some(true)).await;
    for mut body in [
        json!({"items":["short", "界".repeat(2001)],"spec":"relevant","context":"current inventory"}),
        json!({"items":["first", "second"],"spec":"relevant","context":"界".repeat(2001)}),
    ] {
        for top_k in [Value::Null, json!(1)] {
            body["top_k"] = top_k;
            let response = triage(&url, &body).await;
            assert_eq!(response.status(), 400);
            let error: Value = response.json().await.unwrap();
            assert_eq!(
                error["error"],
                "bad request: context and item exceed the endpoint state limit; shorten the evidence"
            );
        }
    }
    assert!(embedding.received_requests().await.unwrap().is_empty());
    assert_eq!(posts(&model).await, 0);
}

#[tokio::test]
async fn serialized_object_order_is_preserved_with_and_without_context() {
    let (url, model, _embedding, _dir) = app(None).await;
    let items: Value = serde_json::from_str(
        r#"[{"id":"a","title":"t"},{"title":"t","id":"a"},{"id":"a","title":"t"}]"#,
    )
    .unwrap();
    for context in [Value::Null, json!("current inventory")] {
        let body = json!({"items":items,"spec":"relevant","context":context});
        let response = triage(&url, &body).await;
        assert_eq!(response.status(), 200);
        let response: Value = response.json().await.unwrap();
        let indices: Vec<_> = response["kept"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["index"].as_u64().unwrap())
            .collect();
        assert_eq!(indices, [0, 1]);
    }
    assert_eq!(posts(&model).await, 4);
}

#[tokio::test]
async fn rejected_decisions_are_distinct_from_unjudged_prefiltered_items() {
    let (url, _model, _embedding, _dir) = app(Some(true)).await;
    let body: Value = triage(&url, &request(&json!(["no", "unsure", "yes1"])))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(body["rejected"][0]["index"], 0);
    assert_eq!(body["rejected"][0]["decision"]["verdict"], "no");
    assert_eq!(body["prefiltered"], json!([]));
    let mut request = request(&json!(["one", "three"]));
    request["top_k"] = json!(1);
    let body: Value = triage(&url, &request).await.json().await.unwrap();
    assert_eq!(body["prefiltered"], json!([0]));
    assert_eq!(body["rejected"], json!([]));
    assert_eq!(body["dropped"], 1);
}

#[tokio::test]
async fn cli_supports_named_spec_and_context_file() {
    let (url, model, _embedding, directory) = app(None).await;
    let context_path = directory.path().join("context.txt");
    std::fs::write(&context_path, "uses kopiur").unwrap();
    cli_triage(
        &[
            "--spec",
            "relevant",
            "--context-file",
            context_path.to_str().unwrap(),
            "--url",
            &url,
        ],
        br#"["backup change"]"#,
    )
    .await;
    let requests = model.received_requests().await.unwrap();
    let call = requests
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&call.body).unwrap();
    let state: Value = serde_json::from_str(body["state"].as_str().unwrap()).unwrap();
    assert_eq!(state["context"], "uses kopiur");
    assert!(body["questions"].get("relevant").is_some());
}
