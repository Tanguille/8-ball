use serde_json::json;
use wiremock::{MockServer, matchers};

use backend::Endpoint;
use domain::{config::EndpointConfig, question::Question};

#[tokio::test]
async fn score_is_mapped_from_logit() {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/rerank"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"index":0,"relevance_score":0.8}]})),
        )
        .mount(&mock)
        .await;
    let endpoint_config: EndpointConfig = toml::from_str(&format!(
        "url='{}'\nkind='rerank'\nmodel='ranker'",
        mock.uri()
    ))
    .unwrap();
    let endpoint = Endpoint::new("rerank", &endpoint_config, None);
    let question: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"disk full"})).unwrap();
    let probabilities = endpoint.ask("disk at 98%", &[question]).await.unwrap();
    assert!((probabilities[0][0] - 0.689_974_481_127_612_5).abs() < 1e-9);
    assert!((probabilities[0][1] - 0.310_025_518_872_387_5).abs() < 1e-9);
    let requests = mock.received_requests().await.unwrap();
    let request = requests
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({"query":"disk full","documents":["disk at 98%"],"model":"ranker"})
    );
}
#[tokio::test]
async fn choice_is_rejected() {
    let mock = MockServer::start().await;
    let endpoint_config: EndpointConfig =
        toml::from_str(&format!("url='{}'\nkind='rerank'", mock.uri())).unwrap();
    let endpoint = Endpoint::new("rerank", &endpoint_config, None);
    let question: Question = serde_json::from_value(
        json!({"id":"q","type":"choice","instructions":"team","criteria":{"a":"","b":""}}),
    )
    .unwrap();
    assert!(
        endpoint
            .ask("hi", &[question])
            .await
            .unwrap_err()
            .to_string()
            .contains("rerank answers noul questions only")
    );
    assert!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method != "POST")
    );
}
