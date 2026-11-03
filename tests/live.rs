use eightball::{backend::Endpoint, config::Config, question::Question, verdict::expected_score};
use serde_json::json;

async fn require(port: u16) {
    let ok = reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .is_ok_and(|r| r.status().is_success());
    assert!(
        ok,
        "model server on :{port} is not up; run dev/models.sh up"
    );
}
fn dev_config() -> Config {
    Config::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dev/config.toml")).unwrap()
}
fn d3() -> Endpoint {
    Endpoint::new("d3", &dev_config().endpoints["d3"], None).unwrap()
}
fn questions() -> Vec<Question> {
    serde_json::from_value(json!([
        {"id":"greeting","type":"noul","instructions":"Is this message a greeting?"},
        {"id":"team","type":"choice","instructions":"Which team should handle this?","criteria":{"billing":"","shipping":"","technical":""}},
        {"id":"urgency","type":"score","instructions":"How urgent is this?","criteria":["can wait","this week","today","right now"]}
    ])).unwrap()
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_noul_clear_cases() {
    require(9931).await;
    let ep = d3();
    let qs = questions();
    for (state, yes) in [
        ("hi there, how are you?", true),
        ("Good morning everyone!", true),
        ("The disk on node control-2 is 98% full.", false),
        ("Invoice #4411 is attached.", false),
    ] {
        let (p, _) = ep.ask(state, &qs[..1]).await.unwrap();
        eprintln!("{state:?}: {p:?}");
        assert!(
            if yes { p[0][0] > 0.8 } else { p[0][0] < 0.2 },
            "{state:?}: {p:?}"
        );
    }
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_choice() {
    require(9931).await;
    let ep = d3();
    let qs = questions();
    for (state, index) in [
        ("I was charged twice for my order", 0),
        ("My parcel never arrived", 1),
        ("The app crashes when I log in", 2),
    ] {
        let (p, _) = ep.ask(state, &qs[1..2]).await.unwrap();
        eprintln!("{state:?}: {p:?}");
        assert!(p[0].iter().all(|v| *v <= p[0][index]), "{state:?}: {p:?}");
    }
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_score() {
    require(9931).await;
    let ep = d3();
    let qs = questions();
    let (high, _) = ep
        .ask(
            "Production database is down, all customers affected",
            &qs[2..],
        )
        .await
        .unwrap();
    let (low, _) = ep
        .ask("Typo in the footer of an internal wiki page", &qs[2..])
        .await
        .unwrap();
    eprintln!(
        "urgent: {high:?}, score {}; typo: {low:?}, score {}",
        expected_score(&high[0]),
        expected_score(&low[0])
    );
    assert!(expected_score(&high[0]) > 2.0);
    assert!(expected_score(&low[0]) < 1.5);
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_three_questions_one_call() {
    require(9931).await;
    let (p, _) = d3()
        .ask(
            "I was charged twice and I'm furious, fix it today",
            &questions(),
        )
        .await
        .unwrap();
    eprintln!("multi: {p:?}");
    assert_eq!(p.len(), 3);
    for probs in p {
        assert!((probs.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    }
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_long_state_fits() {
    require(9931).await;
    let mut state = String::from("hi there, how are you?\n");
    state.push_str(&"log line\n".repeat(2500));
    state.truncate(20_000);
    let (p, sent) = d3().ask(&state, &questions()[..1]).await.unwrap();
    eprintln!(
        "long state: {p:?}; fitted characters {}",
        sent.chars().count()
    );
    assert_eq!(sent.chars().count(), 2003);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_cli_ask() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    require(9931).await;
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_8ball"))
            .args([
                "serve",
                "--config",
                "dev/config.toml",
                "--listen",
                "127.0.0.1:0",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stderr = child.0.stderr.take().unwrap();
    let mut url = None;
    for line in BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if let Some((_, addr)) = line.split_once("listening on ") {
            url = Some(format!("http://{}", addr.trim()));
            break;
        }
    }
    let output = assert_cmd::cargo::cargo_bin_cmd!("8ball")
        .args([
            "ask",
            "noul",
            "Is this message a greeting?",
            "--state",
            "hi there",
            "--exit-code",
            "--url",
            &url.expect("listening address"),
        ])
        .output()
        .unwrap();
    eprintln!("cli: {}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_detect_d3() {
    require(9931).await;
    let ep = d3();
    let info = ep.info().await.unwrap();
    eprintln!("d3 info: {info:?}");
    assert_eq!(info.kind, eightball::config::BackendKind::Systemone);
    assert_eq!(info.concurrency, 2);
    assert!(info.fingerprint.contains("vllm-d3-nano-Q8_0.gguf@b"));
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_detect_chat() {
    require(9934).await;
    let ep = Endpoint::new("chat", &dev_config().endpoints["chat"], None).unwrap();
    let info = ep.info().await.unwrap();
    eprintln!("chat info: {info:?}");
    assert_eq!(info.kind, eightball::config::BackendKind::Logprob);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_logprob_noul() {
    require(9934).await;
    let ep = Endpoint::new("chat", &dev_config().endpoints["chat"], None).unwrap();
    let qs = questions();
    let mut greetings = Vec::new();
    let mut non_greetings = Vec::new();
    for (state, yes) in [
        ("hi there, how are you?", true),
        ("Good morning everyone!", true),
        ("The disk on node control-2 is 98% full.", false),
        ("Invoice #4411 is attached.", false),
    ] {
        let (p, _) = ep.ask(state, &qs[..1]).await.unwrap();
        eprintln!("chat {state:?}: {p:?}");
        assert!(
            if yes { p[0][0] > 0.5 } else { p[0][0] < 0.5 },
            "{state:?}: {p:?}"
        );
        if yes {
            greetings.push(p[0][0]);
        } else {
            non_greetings.push(p[0][0]);
        }
    }
    for greeting in greetings {
        for non_greeting in &non_greetings {
            assert!(
                greeting > *non_greeting,
                "greeting {greeting} <= non-greeting {non_greeting}"
            );
        }
    }
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_logprob_choice() {
    require(9934).await;
    let ep = Endpoint::new("chat", &dev_config().endpoints["chat"], None).unwrap();
    let qs = questions();
    let mut correct = 0;
    for (state, index) in [
        ("I was charged twice for my order", 0),
        ("My parcel never arrived", 1),
        ("The app crashes when I log in", 2),
    ] {
        let (p, _) = ep.ask(state, &qs[1..2]).await.unwrap();
        eprintln!("chat {state:?}: {p:?}");
        if p[0].iter().all(|v| *v <= p[0][index]) {
            correct += 1;
        }
    }
    eprintln!("chat choice correct: {correct}/3");
    assert!(correct >= 2);
}
#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_logprob_mass() {
    require(9934).await;
    let ep = Endpoint::new("chat", &dev_config().endpoints["chat"], None).unwrap();
    let result = ep.ask("hi there, how are you?", &questions()[..1]).await;
    eprintln!("mass validation: {result:?}");
    assert!(result.is_ok());
}
