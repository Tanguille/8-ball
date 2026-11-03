use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Server {
    child: Child,
    _config: tempfile::NamedTempFile,
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
    let mut config = tempfile::Builder::new()
        .prefix("8ball-")
        .tempfile()
        .unwrap();
    writeln!(
        config,
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'",
        mock.uri()
    )
    .unwrap();
    let db = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let db_path = db.path().join("db.sqlite");
    let child = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "serve",
            "--db",
            db_path.to_str().unwrap(),
            "--config",
            config.path().to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut server = Server {
        child,
        _config: config,
        _db: db,
        url: String::new(),
    };
    let stderr = server.child.stderr.take().unwrap();
    for line in BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if let Some((_, addr)) = line.split_once("listening on ") {
            server.url = format!("http://{}", addr.trim());
            return server;
        }
    }
    panic!("server exited before logging listening address");
}
async fn mock(p: f64) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"q":{"type":"noul","noul":p}}})),
        )
        .mount(&mock)
        .await;
    mock
}
#[tokio::test]
async fn noul_exit_codes() {
    for (p, code, want) in [(0.97, 0, "yes"), (0.02, 1, "no"), (0.5, 3, "unsure")] {
        let mock = mock(p).await;
        let server = serve(&mock);
        let output = assert_cmd::cargo::cargo_bin_cmd!("8ball")
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
async fn choice_prints_billing() {
    let mock = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/systemone")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":{"q":{"type":"choice","probabilities":{"billing":0.9,"shipping":0.1}}}}))).mount(&mock).await;
    let server = serve(&mock);
    let output = assert_cmd::cargo::cargo_bin_cmd!("8ball")
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
#[tokio::test]
async fn dead_server_exits_four() {
    let mock = mock(0.97).await;
    let mut server = serve(&mock);
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let output = assert_cmd::cargo::cargo_bin_cmd!("8ball")
        .args([
            "ask",
            "noul",
            "Greeting?",
            "--state",
            "hi",
            "--url",
            &server.url,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stderr.starts_with(b"8ball:"));
}
