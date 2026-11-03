use eightball::{
    backend::Endpoint,
    config::{BackendKind, EndpointConfig},
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn cfg(url: &str, extra: &str) -> EndpointConfig {
    toml::from_str(&format!("url='{url}'\n{extra}")).unwrap()
}
async fn models(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}
#[tokio::test]
async fn decisions_props_and_fingerprint() {
    let server = MockServer::start().await;
    models(
        &server,
        json!({"data":[{"id":"m","architecture":{"output_modalities":["decisions"]}}]}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/props"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"total_slots":4,"model_path":"/m/a.gguf","build_info":"b1-x"}),
            ),
        )
        .mount(&server)
        .await;
    let ep = Endpoint::new("d3", &cfg(&server.uri(), ""), None).unwrap();
    let info = ep.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Systemone);
    assert_eq!(info.concurrency, 4);
    assert_eq!(info.fingerprint, "/m/a.gguf@b1-x");
    ep.info().await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
#[tokio::test]
async fn text_architecture_and_model_detect_logprob() {
    let server = MockServer::start().await;
    models(
        &server,
        json!({"data":[{"id":"m","architecture":{"output_modalities":["text"]}}]}),
    )
    .await;
    let ep = Endpoint::new("chat", &cfg(&server.uri(), "model='m'"), None).unwrap();
    let info = ep.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Logprob);
    assert_eq!(info.fingerprint, "m");
}
#[tokio::test]
async fn missing_architecture_requires_explicit_kind() {
    let server = MockServer::start().await;
    models(&server, json!({"data":[{"id":"m"}]})).await;
    let ep = Endpoint::new("old", &cfg(&server.uri(), ""), None).unwrap();
    assert!(
        ep.info()
            .await
            .unwrap_err()
            .to_string()
            .contains("set kind")
    );
}
#[tokio::test]
async fn explicit_kind_skips_models() {
    let server = MockServer::start().await;
    let ep = Endpoint::new(
        "rerank",
        &cfg(&server.uri(), "kind='rerank'\nconcurrency=1"),
        None,
    )
    .unwrap();
    let info = ep.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Rerank);
    assert_eq!(info.concurrency, 1);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/v1/models")
    );
}
#[tokio::test]
async fn failed_detection_is_not_cached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    models(
        &server,
        json!({"data":[{"id":"m","architecture":{"output_modalities":["decisions"]}}]}),
    )
    .await;
    let ep = Endpoint::new("d3", &cfg(&server.uri(), ""), None).unwrap();
    assert!(ep.info().await.is_err());
    assert_eq!(ep.info().await.unwrap().kind, BackendKind::Systemone);
}
#[tokio::test]
async fn detection_gets_send_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/props"))
        .and(header("authorization", "Bearer secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_slots":1})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"id":"m","architecture":{"output_modalities":["decisions"]}}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let ep = Endpoint::new("d3", &cfg(&server.uri(), ""), Some("secret".into())).unwrap();
    assert_eq!(ep.info().await.unwrap().concurrency, 1);
}
