use serde_json::{Value, json};
use wiremock::{MockServer, matchers};

mod common;

async fn app(key: Option<&str>) -> (String, MockServer, tempfile::TempDir) {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":0.97}}})),
        )
        .mount(&mock)
        .await;
    let endpoint_config: eightball::config::Config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'",
        mock.uri()
    ))
    .unwrap();
    let (service, _, directory) = common::service(endpoint_config).await;

    (common::serve(service, key).await, mock, directory)
}
fn ask() -> Value {
    json!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]})
}
#[tokio::test]
async fn bearer_required_for_rest_and_mcp() {
    let (url, _mock, _dir) = app(Some("k")).await;
    for key in [None, Some("wrong"), Some("k")] {
        let mut request = reqwest::Client::new()
            .post(format!("{url}/v1/ask"))
            .json(&ask());
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        assert_eq!(
            request.send().await.unwrap().status(),
            if key == Some("k") { 200 } else { 401 }
        );
    }
    for path in ["/v1/triage", "/v1/label", "/v1/calibrate", "/mcp"] {
        assert_eq!(
            reqwest::Client::new()
                .post(format!("{url}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        reqwest::get(format!("{url}/v1/doctor"))
            .await
            .unwrap()
            .status(),
        401
    );
}
#[tokio::test]
async fn health_metrics_and_oauth_probes_are_public() {
    let (url, _mock, _dir) = app(Some("k")).await;
    for path in ["/healthz", "/metrics"] {
        assert_eq!(
            reqwest::get(format!("{url}{path}")).await.unwrap().status(),
            200
        );
    }
    let response = reqwest::get(format!("{url}/.well-known/oauth-protected-resource"))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert!(
        response.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("OAuth")
    );
    let response = reqwest::Client::new()
        .post(format!("{url}/register"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert!(response.json::<Value>().await.unwrap()["error"].is_string());
}
#[tokio::test]
async fn cli_passes_key_and_reports_unauthorized_status() {
    let (url, _mock, _dir) = app(Some("k")).await;
    for key in [None, Some("k")] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
        command
            .args(["ask", "noul", "Greeting?", "--state", "hi", "--url", &url])
            .env_remove("EIGHTBALL_API_KEY");
        if let Some(key) = key {
            command.args(["--api-key", key]);
        }
        let output = command.output().await.unwrap();
        assert_eq!(
            output.status.code(),
            Some(if key.is_some() { 0 } else { 4 }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if key.is_none() {
            assert!(String::from_utf8_lossy(&output.stderr).contains("401"));
        }
    }
}
#[tokio::test]
async fn keyless_server_accepts_empty_bearer_header() {
    let (url, _mock, _dir) = app(None).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/ask"))
        .header("authorization", "Bearer ")
        .json(&ask())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}
#[tokio::test]
async fn keyless_server_rejects_other_hosts() {
    let (url, _mock, _dir) = app(None).await;
    let port = url.rsplit_once(':').unwrap().1;
    // A DNS-rebinding page reaches 127.0.0.1 but sends its own host name.
    for (host, status) in [
        ("rebind.example".to_owned(), 403),
        (format!("rebind.example:{port}"), 403),
        (format!("localhost:{port}"), 200),
        (format!("[::1]:{port}"), 200),
    ] {
        let response = reqwest::Client::new()
            .post(format!("{url}/v1/questions"))
            .header("host", &host)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{host}");
    }
    // Public routes stay reachable.
    let health = reqwest::Client::new()
        .get(format!("{url}/healthz"))
        .header("host", "rebind.example")
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
}
