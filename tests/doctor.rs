use eightball::{backend::Endpoint, config::Config, http::router, service::Service};
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn service(default: &str) -> (Service, MockServer) {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"probe":{"type":"noul","noul":0.97}}})),
        )
        .mount(&mock)
        .await;
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = unused.local_addr().unwrap();
    drop(unused);
    let config: Config=toml::from_str(&format!("default='{default}'\n[endpoints.healthy]\nurl='{}'\nkind='systemone'\n[endpoints.dead]\nurl='http://{dead}'",mock.uri())).unwrap();
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, cfg)| (name.clone(), Endpoint::new(name, cfg, None).unwrap()))
        .collect();
    (Service::new(config, endpoints), mock)
}
#[tokio::test]
async fn reports_healthy_probe_and_dead_error() {
    let (service, _mock) = service("healthy").await;
    let report = service.doctor().await;
    let healthy = report
        .endpoints
        .iter()
        .find(|ep| ep.name == "healthy")
        .unwrap();
    assert_eq!(healthy.probe.as_ref().unwrap()["verdict"], "yes");
    assert!(healthy.error.is_none());
    let dead = report
        .endpoints
        .iter()
        .find(|ep| ep.name == "dead")
        .unwrap();
    assert!(dead.error.is_some());
    assert!(report.embedder.is_none());
    assert!(report.specs.is_empty());
}
#[tokio::test]
async fn cli_exit_depends_on_default_endpoint() {
    for (default, want) in [("healthy", 0), ("dead", 4)] {
        let (service, _mock) = service(default).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router(Arc::new(service)))
                .await
                .unwrap();
        });
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args(["doctor", "--url", &url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(want),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["endpoints"].as_array().unwrap().len(), 2);
        task.abort();
    }
}
