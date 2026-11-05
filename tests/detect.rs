use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use eightball::{backend::Endpoint, config::BackendKind};

fn endpoint_config(url: &str, extra: &str) -> eightball::config::EndpointConfig {
    toml::from_str(&format!("url='{url}'\n{extra}")).unwrap()
}
async fn models(server: &MockServer, body: serde_json::Value) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
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
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/props"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"total_slots":4,"model_path":"/m/a.gguf","build_info":"b1-x"}),
            ),
        )
        .mount(&server)
        .await;
    let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri(), ""), None);
    let info = endpoint.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Systemone);
    assert_eq!(info.concurrency, 4);
    assert_eq!(info.fingerprint, "/m/a.gguf@b1-x");
    endpoint.info().await.unwrap();
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
    let endpoint = Endpoint::new("chat", &endpoint_config(&server.uri(), "model='m'"), None);
    let info = endpoint.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Logprob);
    assert_eq!(info.fingerprint, "m");
}
#[tokio::test]
async fn missing_architecture_requires_explicit_kind() {
    let server = MockServer::start().await;
    models(&server, json!({"data":[{"id":"m"}]})).await;
    let endpoint = Endpoint::new("old", &endpoint_config(&server.uri(), ""), None);
    assert!(
        endpoint
            .info()
            .await
            .unwrap_err()
            .to_string()
            .contains("set kind")
    );
}
#[tokio::test]
async fn explicit_kind_skips_models() {
    let server = MockServer::start().await;
    let endpoint = Endpoint::new(
        "rerank",
        &endpoint_config(&server.uri(), "kind='rerank'\nconcurrency=1"),
        None,
    );
    let info = endpoint.info().await.unwrap();
    assert_eq!(info.kind, BackendKind::Rerank);
    assert_eq!(info.concurrency, 1);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != "/v1/models")
    );
}
#[tokio::test]
async fn failed_detection_is_not_cached() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
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
    let endpoint = Endpoint::new("d3", &endpoint_config(&server.uri(), ""), None);
    assert!(endpoint.info().await.is_err());
    assert_eq!(endpoint.info().await.unwrap().kind, BackendKind::Systemone);
}
#[tokio::test]
async fn detection_gets_send_bearer() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/props"))
        .and(matchers::header("authorization", "Bearer secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_slots":1})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
        .and(matchers::header("authorization", "Bearer secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"id":"m","architecture":{"output_modalities":["decisions"]}}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = Endpoint::new(
        "d3",
        &endpoint_config(&server.uri(), ""),
        Some("secret".into()),
    );
    assert_eq!(endpoint.info().await.unwrap().concurrency, 1);
}
