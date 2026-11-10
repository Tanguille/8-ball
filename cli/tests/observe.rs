use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

struct Fixture {
    directory: tempfile::TempDir,
    snapshot: Value,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("8ball-observe-")
            .tempdir()
            .unwrap();
        let snapshot = json!({"context":"uses kopiur","items":[{"id":"a","title":"backup change"},{"id":"b","title":"routine bump"}]});
        fs::write(directory.path().join("input.json"), snapshot.to_string()).unwrap();

        Self {
            directory,
            snapshot,
        }
    }

    async fn run(&self, url: &str, extra: &[&str]) -> Value {
        // A child-only umask makes the privacy check independent of the test runner's umask.
        let output = Command::new("sh")
            .args(["-c", "umask 000; exec \"$@\"", "8ball-test"])
            .arg(env!("CARGO_BIN_EXE_8ball"))
            .args([
                "observe",
                "--spec",
                "gitops-investigate",
                "--url",
                url,
                "--api-key",
                "test-key",
                "--input",
            ])
            .arg(self.directory.path().join("input.json"))
            .arg("--observations")
            .arg(self.directory.path().join("run.jsonl"))
            .args(extra)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["wakeAgent"], true);

        body
    }

    fn record(&self) -> Value {
        let text = fs::read_to_string(self.directory.path().join("run.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 1, "one JSONL record per run");
        assert!(text.ends_with('\n'), "JSONL record ends with a newline");

        serde_json::from_str(&text).unwrap()
    }
}

async fn server(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/triage"))
        .respond_with(response)
        .mount(&server)
        .await;

    server
}

#[tokio::test]
async fn observes_without_filtering_and_saves_private_record() {
    let response = json!({"kept":[],"unsure":[],"rejected":[{"index":0},{"index":1}],"prefiltered":[],"dropped":2});
    let server = server(ResponseTemplate::new(200).set_body_json(response.clone())).await;
    let fixture = Fixture::new();
    let body = fixture
        .run(&server.uri(), &["--endpoint", "candidate"])
        .await;
    assert_eq!(body["context"]["snapshot"], fixture.snapshot);
    assert!(body["context"].get("result").is_none());
    let record = fixture.record();
    assert_eq!(record["mode"], "observe");
    assert_eq!(record["snapshot"], fixture.snapshot);
    assert_eq!(record["result"], response);
    assert!(record["error"].is_null());
    assert!(record["observed_at_unix_ms"].as_u64().unwrap() > 0);
    assert_eq!(
        fs::metadata(fixture.directory.path().join("run.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer test-key"
    );
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["items"], fixture.snapshot["items"]);
    assert_eq!(sent["context"], fixture.snapshot["context"]);
    assert_eq!(sent["endpoint"], "candidate");
    assert_eq!(sent["spec"], "gitops-investigate");
    assert!(sent.get("top_k").is_none());
}

#[tokio::test]
async fn backend_errors_malformed_json_and_timeouts_keep_original_snapshot() {
    for response in [
        ResponseTemplate::new(502).set_body_json(json!({"error":"unavailable"})),
        ResponseTemplate::new(200).set_body_string("broken json"),
        ResponseTemplate::new(200)
            .set_body_json(json!({}))
            .set_delay(Duration::from_secs(10)),
    ] {
        let server = server(response).await;
        let fixture = Fixture::new();
        let body = fixture.run(&server.uri(), &["--timeout-ms", "100"]).await;
        assert_eq!(body["context"]["snapshot"], fixture.snapshot);
        assert!(body["context"]["decision_error"].is_string());
        assert!(fixture.record()["error"].is_string());
    }
}

#[tokio::test]
async fn refuses_redirects_without_forwarding_credentials() {
    let destination = MockServer::start().await;
    let server = server(
        ResponseTemplate::new(302)
            .insert_header("Location", destination.uri())
            .set_body_json(json!({})),
    )
    .await;
    let fixture = Fixture::new();
    let body = fixture.run(&server.uri(), &[]).await;
    assert_eq!(body["context"]["snapshot"], fixture.snapshot);
    assert!(fixture.record()["error"].as_str().unwrap().contains("302"));
    assert!(destination.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn truncated_response_still_wakes_agent() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        assert!(stream.read(&mut buffer).await.unwrap() > 0);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 99999\r\nConnection: close\r\n\r\n{}").await.unwrap();
    });
    let fixture = Fixture::new();
    let body = fixture.run(&url, &[]).await;
    assert_eq!(body["context"]["snapshot"], fixture.snapshot);
    let record = fixture.record();
    assert!(record["error"].as_str().unwrap().contains("body"));
    assert!(record["result"].is_null());
    task.await.unwrap();
}

#[tokio::test]
async fn existing_observation_file_is_preserved() {
    let server = server(ResponseTemplate::new(200).set_body_json(json!({}))).await;
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("run.jsonl");
    fs::write(&path, "previous run\n").unwrap();
    let body = fixture.run(&server.uri(), &[]).await;
    assert_eq!(body["context"]["snapshot"], fixture.snapshot);
    assert!(body["context"]["observation_error"].is_string());
    assert_eq!(fs::read_to_string(path).unwrap(), "previous run\n");
}

#[tokio::test]
async fn invalid_or_missing_input_wakes_agent_without_inference() {
    let server = server(ResponseTemplate::new(200)).await;
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("input.json");
    for input in [
        "not json",
        "{}",
        "{\"context\":1,\"items\":[]}",
        "{\"context\":\"ok\",\"items\":null}",
    ] {
        fs::write(&path, input).unwrap();
        let body = fixture.run(&server.uri(), &[]).await;
        assert!(body["context"]["input_error"].is_string());
        assert!(body["context"].get("snapshot").is_none());
        assert!(!fixture.directory.path().join("run.jsonl").exists());
    }
    fs::remove_file(path).unwrap();
    let body = fixture.run(&server.uri(), &[]).await;
    assert!(body["context"]["input_error"].is_string());
    assert!(body["context"].get("snapshot").is_none());
    assert!(!fixture.directory.path().join("run.jsonl").exists());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn calibration_is_not_limited_by_the_decision_deadline() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/calibrate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"stored":true}))
                // Recalibration may re-infer many labelled states after a model change.
                .set_delay(Duration::from_secs(26)),
        )
        .mount(&server)
        .await;
    let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["calibrate", "relevant", "--url", &server.uri()])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["stored"], true);
}
