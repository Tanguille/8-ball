use eightball::{backend::Endpoint, config::EndpointConfig, question::Question};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn score_is_mapped_from_logit() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/rerank"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"index":0,"relevance_score":0.8}]})),
        )
        .mount(&mock)
        .await;
    let cfg: EndpointConfig = toml::from_str(&format!(
        "url='{}'\nkind='rerank'\nmodel='ranker'",
        mock.uri()
    ))
    .unwrap();
    let ep = Endpoint::new("rerank", &cfg, None).unwrap();
    let q: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"disk full"})).unwrap();
    let p = ep.ask("disk at 98%", &[q]).await.unwrap();
    assert!((p[0][0] - 0.689_974_481_127_612_5).abs() < 1e-9);
    assert!((p[0][1] - 0.310_025_518_872_387_5).abs() < 1e-9);
    let requests = mock.received_requests().await.unwrap();
    let request = requests.iter().find(|r| r.method == "POST").unwrap();
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({"query":"disk full","documents":["disk at 98%"],"model":"ranker"})
    );
}
#[tokio::test]
async fn choice_is_rejected() {
    let mock = MockServer::start().await;
    let cfg: EndpointConfig =
        toml::from_str(&format!("url='{}'\nkind='rerank'", mock.uri())).unwrap();
    let ep = Endpoint::new("rerank", &cfg, None).unwrap();
    let q: Question = serde_json::from_value(
        json!({"id":"q","type":"choice","instructions":"team","criteria":{"a":"","b":""}}),
    )
    .unwrap();
    assert!(
        ep.ask("hi", &[q])
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
            .all(|r| r.method != "POST")
    );
}
