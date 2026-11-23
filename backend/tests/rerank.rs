use serde_json::json;
use wiremock::{MockServer, matchers};

use backend::Endpoint;
use domain::question::Question;

use common::posts;

#[path = "utils/common.rs"]
mod common;

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
    let config = common::config(&mock.uri(), "kind='rerank'\nmodel='ranker'");
    let endpoint = Endpoint::new("rerank", &config, None);
    let question: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"disk full"})).unwrap();
    let probabilities = endpoint.ask("disk at 98%", &[question]).await.unwrap();
    assert!((probabilities[0][0] - 0.689_974_481_127_612_5).abs() < 1e-9);
    assert!((probabilities[0][1] - 0.310_025_518_872_387_5).abs() < 1e-9);
    let body: serde_json::Value = serde_json::from_slice(&posts(&mock).await[0].body).unwrap();
    assert_eq!(
        body,
        json!({"query":"disk full","documents":["disk at 98%"],"model":"ranker"})
    );
}
#[tokio::test]
async fn choice_is_rejected() {
    let mock = MockServer::start().await;
    let config = common::config(&mock.uri(), "kind='rerank'");
    let endpoint = Endpoint::new("rerank", &config, None);
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
    assert!(posts(&mock).await.is_empty());
}
