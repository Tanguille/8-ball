use std::io::BufRead;

use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

struct Server {
    child: std::process::Child,
    _db: tempfile::TempDir,
    url: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn serve(mock: &MockServer) -> Server {
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let database_path = directory.path().join("db.sqlite");
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "serve",
            "--db",
            database_path.to_str().unwrap(),
            "--config-toml",
            &format!(
                "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'",
                mock.uri()
            ),
            "--listen",
            "127.0.0.1:0",
        ])
        // A config path from the developer's shell would conflict with --config-toml.
        .env_remove("EIGHTBALL_CONFIG")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut server = Server {
        child,
        _db: directory,
        url: String::new(),
    };
    let stderr = server.child.stderr.take().unwrap();
    for line in std::io::BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if let Some((_, address)) = line.split_once("listening on ") {
            server.url = format!("http://{}", address.trim());

            return server;
        }
    }
    panic!("server exited before logging listening address");
}
async fn mock(probabilities: f64) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":probabilities}}})),
        )
        .mount(&mock)
        .await;

    mock
}
#[tokio::test]
async fn noul_exit_codes() {
    for (probabilities, code, want) in [(0.97, 0, "yes"), (0.02, 1, "no"), (0.5, 3, "unsure")] {
        let mock = mock(probabilities).await;
        let server = serve(&mock);
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args([
                "ask",
                "noul",
                "Is this a greeting?",
                "--state",
                "hi",
                "--exit-code",
                "--url",
                &server.url,
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["results"][0]["verdict"], want);
    }
}
#[tokio::test]
async fn spec_exit_codes_do_not_probe_doctor() {
    for (kind, code) in [("noul", 1), ("choice", 0)] {
        let mock = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/ask"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"results":[{"type":kind,"verdict":"no"}]})),
            )
            .expect(1)
            .mount(&mock)
            .await;
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args([
                "ask",
                "--spec",
                "named",
                "--state",
                "hi",
                "--exit-code",
                "--url",
                &mock.uri(),
            ])
            .output()
            .await
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    }
}
#[tokio::test]
async fn choice_prints_billing() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":{"q":{"type":"choice","probabilities":{"billing":0.9,"shipping":0.1}}}})))
        .mount(&mock)
        .await;
    let server = serve(&mock);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "ask",
            "choice",
            "Team?",
            "--option",
            "billing",
            "--option",
            "shipping",
            "--state",
            "x",
            "--url",
            &server.url,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["results"][0]["verdict"], "billing");
}
#[test]
fn dead_server_exits_four() {
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = unused.local_addr().unwrap();
    drop(unused);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "ask",
            "noul",
            "Greeting?",
            "--state",
            "hi",
            "--url",
            &format!("http://{dead}"),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stderr.starts_with(b"8ball:"));
}
