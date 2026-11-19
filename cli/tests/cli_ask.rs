use std::process::Command;

use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

#[path = "../../service/tests/common.rs"]
mod common;
mod server;

fn serve(mock: &MockServer) -> server::Server {
    let config = format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[learning]\nexplore=0.0",
        mock.uri()
    );

    server::serve(&["--config-toml", &config])
}
#[tokio::test]
async fn noul_exit_codes() {
    let mock = common::systemone().await;
    let server = serve(&mock);
    for (state, code, want) in [
        ("yes-1", 0, "yes"),
        ("no-1", 1, "no"),
        ("maybe-1", 3, "unsure"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args([
                "ask",
                "noul",
                "Is this a greeting?",
                "--state",
                state,
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
async fn no_learn_sends_learn_false() {
    let mock = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/ask"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[]})))
        .mount(&mock)
        .await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["ask", "--spec", "named", "--state", "hi", "--no-learn"])
        .args(["--url", &mock.uri()])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(common::requests(&mock).await[0]["learn"], false);
}
#[tokio::test]
async fn choice_prints_billing() {
    let mock = common::systemone().await;
    let server = serve(&mock);
    let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
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
#[test]
fn invalid_arguments_exit_two() {
    // A missing config and a closed port make a command that gets past clap exit 4, not hang.
    let dead = "http://127.0.0.1:9";
    for args in [
        &[
            "serve",
            "--api-key",
            "",
            "--config",
            "/nonexistent/8ball.toml",
        ][..],
        &[
            "ask",
            "--spec",
            "named",
            "noul",
            "Greeting?",
            "--state",
            "hi",
            "--url",
            dead,
        ],
        &[
            "ask", "--spec", "named", "--option", "a", "--state", "hi", "--url", dead,
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args(args)
            .env_remove("EIGHTBALL_CONFIG_TOML")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
