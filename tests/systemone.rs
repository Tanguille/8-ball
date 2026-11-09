use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use wiremock::{MockServer, ResponseTemplate, matchers};

use eightball::{backend::Endpoint, question::Question};

fn question() -> Question {
    serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Is this a greeting?"}))
        .unwrap()
}
fn endpoint_config(url: &str) -> eightball::config::EndpointConfig {
    toml::from_str(&format!("url = '{url}'\nkind = 'systemone'\n")).unwrap()
}
fn response() -> Value {
    json!({"answers":{"q":{"type":"noul","noul":0.97}}})
}
async fn mount(server: &MockServer, template: ResponseTemplate) {
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(template)
        .mount(server)
        .await;
}

#[tokio::test]
async fn three_questions_one_request() {
    let server = MockServer::start().await;
    let test_questions: Vec<Question>  =  serde_json::from_value(json!([
        {"id":"q1","type":"noul","instructions":"Is this a greeting?"},
        {"id":"q2","type":"choice","instructions":"Which team?","criteria":{"billing":"","shipping":""}},
        {"id":"q3","type":"score","instructions":"How urgent?","criteria":["can wait","today","now"]}
    ])).unwrap();
    mount(&server, ResponseTemplate::new(200).set_body_json(json!({"model":"vllm-d3","answers":{
        "q1":{"type":"noul","noul":0.97},
        "q2":{"type":"choice","choice":"billing","probabilities":{"billing":0.9,"shipping":0.1},"confidence":0.8},
        "q3":{"type":"score","score":1.2,"probabilities":{"0":0.1,"1":0.6,"2":0.3},"confidence":0.5}
    }}))).await;
    let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri()), None);
    let got = endpoint.ask("hello", &test_questions).await.unwrap();
    for (actual, expected) in
        got.iter()
            .zip([vec![0.97, 0.03], vec![0.9, 0.1], vec![0.1, 0.6, 0.3]])
    {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-9);
        }
    }
    let requests: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["questions"]["q1"]["type"], "noul");
    assert!(
        body["questions"]["q2"]["criteria"]
            .as_object()
            .unwrap()
            .contains_key("billing")
    );
    assert!(body["questions"]["q2"]["criteria"]["billing"].is_null());
}

#[tokio::test]
async fn long_state_is_fitted() {
    let server = MockServer::start().await;
    mount(
        &server,
        ResponseTemplate::new(200).set_body_json(response()),
    )
    .await;
    let mut config = endpoint_config(&server.uri());
    config.max_state_chars = Some(10);
    let endpoint = Endpoint::new("d3", &config, None);
    let info = endpoint.info().await.unwrap();
    let sent = endpoint.fitted(&info, "abcdefghijklmnopqrstuvwxyz0123");
    assert_eq!(sent, "abcde\n…\nz0123");
    endpoint.ask(&sent, &[question()]).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let request = requests
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["state"], "abcde\n…\nz0123");
}
#[test]
fn fit_respects_char_boundaries() {
    assert_eq!(
        eightball::backend::fit(&"é".repeat(30), Some(10)),
        format!("{}\n…\n{}", "é".repeat(5), "é".repeat(5))
    );
    assert_eq!(eightball::backend::fit("hi", Some(10)), "hi");
    assert_eq!(eightball::backend::fit("hello", None), "hello");
}
#[tokio::test]
async fn http_400_is_error_with_body() {
    let server = MockServer::start().await;
    mount(
        &server,
        ResponseTemplate::new(400).set_body_json(json!({"error":"too long"})),
    )
    .await;
    let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri()), None);
    let err = endpoint
        .ask("hi", &[question()])
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("400") && err.contains("too long"), "{err}");
}
#[tokio::test]
async fn http_501_is_not_decision_model() {
    let server = MockServer::start().await;
    mount(&server, ResponseTemplate::new(501)).await;
    let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri()), None);
    assert!(
        endpoint
            .ask("hi", &[question()])
            .await
            .unwrap_err()
            .to_string()
            .contains("not a decision model")
    );
}
#[tokio::test]
async fn bearer_only_when_set() {
    for key in [None, Some("secret".into())] {
        let server = MockServer::start().await;
        mount(
            &server,
            ResponseTemplate::new(200).set_body_json(response()),
        )
        .await;
        let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri()), key.clone());
        endpoint.ask("hi", &[question()]).await.unwrap();
        let requests: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.method == "POST")
            .collect();
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .map(|value| value.to_str().unwrap()),
            key.as_ref().map(|_| "Bearer secret")
        );
    }
}
#[tokio::test]
async fn concurrency_one_serializes() {
    for concurrency in [1, 2] {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let router = axum::Router::new().route(
            "/v1/systemone",
            axum::routing::post({
                let active = active.clone();
                let peak = peak.clone();
                move || {
                    let active = active.clone();
                    let peak = peak.clone();
                    let barrier = barrier.clone();
                    async move {
                        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(count, Ordering::SeqCst);
                        if concurrency == 2 {
                            // Both handlers must be in flight; serial execution times out instead.
                            barrier.wait().await;
                        } else {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        active.fetch_sub(1, Ordering::SeqCst);

                        axum::Json(response())
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut config = endpoint_config(&url);
        config.concurrency = Some(concurrency);
        let endpoint = Endpoint::new("d3", &config, None);
        let test_questions = [question()];
        let (a, b) = tokio::join!(
            endpoint.ask("hi", &test_questions),
            endpoint.ask("hi", &test_questions)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), concurrency);
        task.abort();
    }
}

#[tokio::test]
async fn invalid_backend_probabilities_are_rejected() {
    for value in [
        json!({"noul":2.0}),
        json!({"noul":-0.1}),
        json!({"noul":null}),
        json!({"probabilities":{"a":-0.2,"b":1.0}}),
        json!({"probabilities":{"a":0.5,"b":1.2}}),
    ] {
        let server = MockServer::start().await;
        mount(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({"answers":{"q":value}})),
        )
        .await;
        let question = if value.get("noul").is_some() {
            question()
        } else {
            serde_json::from_value(json!({"id":"q","type":"choice","instructions":"Which?","criteria":{"a":"","b":""}})).unwrap()
        };
        let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri()), None);
        assert!(
            endpoint.ask("hi", &[question]).await.is_err(),
            "accepted {value}"
        );
    }
}
