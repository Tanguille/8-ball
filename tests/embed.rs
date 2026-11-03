use eightball::{
    backend::embed::{Embedder, cosine},
    config::EmbedderConfig,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

fn embedder(mock: &MockServer) -> Embedder {
    Embedder::new(
        &EmbedderConfig {
            url: mock.uri(),
            model: "embed".into(),
            api_key_env: None,
        },
        None,
    )
}
#[tokio::test]
async fn ten_texts_use_two_ordered_batches() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(|r: &Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let data: Vec<_> = body["input"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, _)| json!({"index":i,"embedding":[1.0,0.0]}))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({"data":data}))
        })
        .mount(&mock)
        .await;
    let result = embedder(&mock)
        .embed(&(0..10).map(|i| i.to_string()).collect::<Vec<_>>())
        .await
        .unwrap();
    assert_eq!(result.len(), 10);
    let requests = mock.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(first["input"].as_array().unwrap().len(), 8);
    assert_eq!(second["input"], json!(["8", "9"]));
    assert_eq!(first["model"], "embed");
}
#[tokio::test]
async fn shuffled_indices_restore_order() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}),
        ))
        .mount(&mock)
        .await;
    let vectors = embedder(&mock)
        .embed(&["a".into(), "b".into()])
        .await
        .unwrap();
    assert!(cosine(&vectors[0], &[1.0, 0.0]) > 0.999);
    assert!(cosine(&vectors[1], &[0.0, 1.0]) > 0.999);
}
#[tokio::test]
async fn mixed_dimensions_rejected() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"index":0,"embedding":[1.0,0.0]},{"index":1,"embedding":[1.0]}]}),
        ))
        .mount(&mock)
        .await;
    assert!(
        embedder(&mock)
            .embed(&["a".into(), "b".into()])
            .await
            .unwrap_err()
            .to_string()
            .contains("embedder returned mixed dimensions")
    );
}
#[test]
fn cosine_similarity() {
    assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < f32::EPSILON);
    assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < f32::EPSILON);
    assert!(cosine(&[0.0, 0.0], &[1.0, 0.0]).abs() < f32::EPSILON);
}
#[tokio::test]
async fn missing_or_duplicate_indices_are_errors() {
    for data in [
        json!([{"index":0,"embedding":[1.0]}]),
        json!([{"index":0,"embedding":[1.0]},{"index":0,"embedding":[2.0]}]),
    ] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":data})))
            .mount(&mock)
            .await;
        assert!(
            embedder(&mock)
                .embed(&["a".into(), "b".into()])
                .await
                .is_err()
        );
    }
}
