use eightball::{
    backend::{Endpoint, fit},
    config::EndpointConfig,
    question::Question,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn question() -> Question {
    serde_json::from_value(json!({"id":"q","type":"noul","instructions":"Is this a greeting?"}))
        .unwrap()
}
fn cfg(url: &str) -> EndpointConfig {
    toml::from_str(&format!("url = '{url}'\nkind = 'systemone'\n")).unwrap()
}
fn response() -> Value {
    json!({"answers":{"q":{"type":"noul","noul":0.97}}})
}
async fn mount(server: &MockServer, template: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(template)
        .mount(server)
        .await;
}

#[tokio::test]
async fn three_questions_one_request() {
    let server = MockServer::start().await;
    let qs: Vec<Question> = serde_json::from_value(json!([
        {"id":"q1","type":"noul","instructions":"Is this a greeting?"},
        {"id":"q2","type":"choice","instructions":"Which team?","criteria":{"billing":"","shipping":""}},
        {"id":"q3","type":"score","instructions":"How urgent?","criteria":["can wait","today","now"]}
    ])).unwrap();
    mount(&server, ResponseTemplate::new(200).set_body_json(json!({"model":"vllm-d3","answers":{
        "q1":{"type":"noul","noul":0.97},
        "q2":{"type":"choice","choice":"billing","probabilities":{"billing":0.9,"shipping":0.1},"confidence":0.8},
        "q3":{"type":"score","score":1.2,"probabilities":{"0":0.1,"1":0.6,"2":0.3},"confidence":0.5}
    }}))).await;
    let ep = Endpoint::new("d3", &cfg(&server.uri()), None).unwrap();
    let (got, _) = ep.ask("hello", &qs).await.unwrap();
    for (actual, expected) in
        got.iter()
            .zip([vec![0.97, 0.03], vec![0.9, 0.1], vec![0.1, 0.6, 0.3]])
    {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-9);
        }
    }
    let requests = server.received_requests().await.unwrap();
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
    let mut config = cfg(&server.uri());
    config.max_state_chars = Some(10);
    let ep = Endpoint::new("d3", &config, None).unwrap();
    let (_, sent) = ep
        .ask("abcdefghijklmnopqrstuvwxyz0123", &[question()])
        .await
        .unwrap();
    assert_eq!(sent, "abcde\n…\nz0123");
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["state"], "abcde\n…\nz0123");
}
#[test]
fn fit_respects_char_boundaries() {
    assert_eq!(
        fit(&"é".repeat(30), Some(10)),
        format!("{}\n…\n{}", "é".repeat(5), "é".repeat(5))
    );
    assert_eq!(fit("hi", Some(10)), "hi");
    assert_eq!(fit("hello", None), "hello");
}
#[tokio::test]
async fn http_400_is_error_with_body() {
    let server = MockServer::start().await;
    mount(
        &server,
        ResponseTemplate::new(400).set_body_json(json!({"error":"too long"})),
    )
    .await;
    let ep = Endpoint::new("d3", &cfg(&server.uri()), None).unwrap();
    let err = ep.ask("hi", &[question()]).await.unwrap_err().to_string();
    assert!(err.contains("400") && err.contains("too long"), "{err}");
}
#[tokio::test]
async fn http_501_is_not_decision_model() {
    let server = MockServer::start().await;
    mount(&server, ResponseTemplate::new(501)).await;
    let ep = Endpoint::new("d3", &cfg(&server.uri()), None).unwrap();
    assert!(
        ep.ask("hi", &[question()])
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
        let mut config = cfg(&server.uri());
        if key.is_some() {
            config.api_key_env = Some("EIGHTBALL_TEST_KEY".into());
        }
        let ep = Endpoint::new("d3", &config, key.clone()).unwrap();
        ep.ask("hi", &[question()]).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests[0]
                .headers
                .get("authorization")
                .map(|v| v.to_str().unwrap()),
            key.as_ref().map(|_| "Bearer secret")
        );
    }
}
#[tokio::test]
async fn concurrency_one_serializes() {
    for concurrency in [1, 2] {
        let server = MockServer::start().await;
        mount(
            &server,
            ResponseTemplate::new(200)
                .set_body_json(response())
                .set_delay(Duration::from_millis(200)),
        )
        .await;
        let mut config = cfg(&server.uri());
        config.concurrency = Some(concurrency);
        let ep = Endpoint::new("d3", &config, None).unwrap();
        let qs = [question()];
        let start = Instant::now();
        let (a, b) = tokio::join!(ep.ask("hi", &qs), ep.ask("hi", &qs));
        a.unwrap();
        b.unwrap();
        if concurrency == 1 {
            assert!(start.elapsed() >= Duration::from_millis(400));
        } else {
            assert!(start.elapsed() < Duration::from_millis(380));
        }
    }
}
