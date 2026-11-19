#[path = "../../service/tests/common.rs"]
mod common;

async fn service(
    default: &str,
) -> (
    std::sync::Arc<service::Service>,
    wiremock::MockServer,
    tempfile::TempDir,
) {
    let mock = common::systemone().await;
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = unused.local_addr().unwrap();
    drop(unused);
    let config: domain::config::Config =
        toml::from_str(
            &format!(
                "default='{default}'\n[endpoints.healthy]\nurl='{}'\nkind='systemone'\n[endpoints.dead]\nurl='http://{dead}'",
                mock.uri(),
            ),
        )
        .unwrap();
    let (service, _, directory) = common::service(config).await;

    (service, mock, directory)
}
#[tokio::test]
async fn reports_healthy_probe_and_dead_error() {
    let (service, _mock, _dir) = service("healthy").await;
    let report = service.doctor().await;
    let healthy = report
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "healthy")
        .unwrap();
    assert_eq!(healthy.probe.as_ref().unwrap()["verdict"], "yes");
    assert!(healthy.error.is_none());
    let dead = report
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "dead")
        .unwrap();
    assert!(dead.error.is_some());
    assert!(report.embedder.is_none());
    assert!(report.specs.is_empty());
}
#[tokio::test]
async fn cli_exit_depends_on_default_endpoint() {
    for (default, want) in [("healthy", 0), ("dead", 4)] {
        let (service, _mock, _dir) = service(default).await;
        let url = common::serve(service, None).await;
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args(["doctor", "--url", &url])
            .output()
            .await
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(want),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["endpoints"].as_array().unwrap().len(), 2);
    }
}
