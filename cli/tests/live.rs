use std::{
    fs,
    io::{BufRead, Read},
    path::Path,
    process::Command,
};

use rmcp::{ServiceExt, model::CallToolRequestParams, object, transport::TokioChildProcess};
use serde_json::json;
use tempfile::Builder;

use backend::{Endpoint, embed};
use domain::{config::Config, question::Question, verdict};
use service::Service;
use store::Store;

async fn require(port: u16) {
    let healthy = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success());
    assert!(
        healthy,
        "model server on :{port} is not up; run dev/models.sh up"
    );
}

fn dev_config_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev/config.toml")
}

fn dev_config() -> Config {
    Config::load(&dev_config_path()).unwrap()
}

fn api_key(variable: Option<&str>) -> Option<String> {
    variable.and_then(|name| std::env::var(name).ok())
}

fn endpoint(name: &str, config: &domain::config::EndpointConfig) -> Endpoint {
    Endpoint::new(name, config, api_key(config.api_key_env.as_deref()))
}

fn d3() -> Endpoint {
    endpoint("d3", &dev_config().endpoints["d3"])
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
    let endpoint = d3();
    let test_questions = questions();
    for (state, yes) in [
        ("hi there, how are you?", true),
        ("Good morning everyone!", true),
        ("The disk on node control-2 is 98% full.", false),
        ("Invoice #4411 is attached.", false),
    ] {
        let probabilities = endpoint.ask(state, &test_questions[..1]).await.unwrap();
        eprintln!("{state:?}: {probabilities:?}");
        assert!(
            if yes {
                probabilities[0][0] > 0.8
            } else {
                probabilities[0][0] < 0.2
            },
            "{state:?}: {probabilities:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_choice() {
    require(9931).await;
    let endpoint = d3();
    let test_questions = questions();
    for (state, index) in [
        ("I was charged twice for my order", 0),
        ("My parcel never arrived", 1),
        ("The app crashes when I log in", 2),
    ] {
        let probabilities = endpoint.ask(state, &test_questions[1..2]).await.unwrap();
        eprintln!("{state:?}: {probabilities:?}");
        assert!(
            probabilities[0]
                .iter()
                .all(|value| *value <= probabilities[0][index]),
            "{state:?}: {probabilities:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_score() {
    require(9931).await;
    let endpoint = d3();
    let test_questions = questions();
    let high = endpoint
        .ask(
            "Production database is down, all customers affected",
            &test_questions[2..],
        )
        .await
        .unwrap();
    let low = endpoint
        .ask(
            "Typo in the footer of an internal wiki page",
            &test_questions[2..],
        )
        .await
        .unwrap();
    eprintln!(
        "urgent: {high:?}, score {}; typo: {low:?}, score {}",
        verdict::expected_score(&high[0]),
        verdict::expected_score(&low[0])
    );
    assert!(verdict::expected_score(&high[0]) > 2.0);
    assert!(verdict::expected_score(&low[0]) < 1.5);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_three_questions_one_call() {
    require(9931).await;
    let probabilities = d3()
        .ask(
            "I was charged twice and I'm furious, fix it today",
            &questions(),
        )
        .await
        .unwrap();
    eprintln!("multi: {probabilities:?}");
    assert_eq!(probabilities.len(), 3);
    for probabilities in probabilities {
        assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    }
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_systemone_long_state_fits() {
    require(9931).await;
    let mut state = String::from("hi there, how are you?\n");
    state.push_str(&"log line\n".repeat(2500));
    state.truncate(20_000);
    let endpoint = d3();
    let info = endpoint.info().await.unwrap();
    let sent = endpoint.fitted(&info, &state);
    let probabilities = endpoint.ask(&sent, &questions()[..1]).await.unwrap();
    eprintln!(
        "long state: {probabilities:?}; fitted characters {}",
        sent.chars().count()
    );
    assert_eq!(sent.chars().count(), 2003);
    assert_eq!(probabilities.len(), 1);
    assert!((probabilities[0].iter().sum::<f64>() - 1.0).abs() < 1e-6);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_cli_ask() {
    require(9931).await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let config = dev_config_path();
    let server = live_server(&config, &directory.path().join("db.sqlite"), None);
    let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "ask",
            "noul",
            "Is this message a greeting?",
            "--state",
            "hi there",
            "--exit-code",
            "--url",
            &server.url,
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
    let endpoint = d3();
    let info = endpoint.info().await.unwrap();
    eprintln!("d3 info: {info:?}");
    assert_eq!(info.kind, domain::config::BackendKind::Systemone);
    assert_eq!(info.concurrency, 2);
    // The model file alone, without the llama.cpp build.
    assert!(
        info.fingerprint.ends_with("/vllm-d3-nano-Q8_0.gguf"),
        "{}",
        info.fingerprint
    );
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_detect_chat() {
    require(9934).await;
    let endpoint = endpoint("chat", &dev_config().endpoints["chat"]);
    let info = endpoint.info().await.unwrap();
    eprintln!("chat info: {info:?}");
    assert_eq!(info.kind, domain::config::BackendKind::Logprob);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_logprob_noul() {
    require(9934).await;
    let endpoint = endpoint("chat", &dev_config().endpoints["chat"]);
    let test_questions = questions();
    let mut greetings = Vec::new();
    let mut non_greetings = Vec::new();
    for (state, yes) in [
        ("hi there, how are you?", true),
        ("Good morning everyone!", true),
        ("The disk on node control-2 is 98% full.", false),
        // Not "Invoice #4411 is attached.": Qwen3.5 0.8B gives it P(yes) 0.486 on the GPU and 0.510 on the CPU.
        ("The build failed with exit code 2.", false),
    ] {
        let probabilities = endpoint.ask(state, &test_questions[..1]).await.unwrap();
        eprintln!("chat {state:?}: {probabilities:?}");
        assert!(
            if yes {
                probabilities[0][0] > 0.5
            } else {
                probabilities[0][0] < 0.5
            },
            "{state:?}: {probabilities:?}"
        );
        if yes {
            greetings.push(probabilities[0][0]);
        } else {
            non_greetings.push(probabilities[0][0]);
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
    let endpoint = endpoint("chat", &dev_config().endpoints["chat"]);
    let test_questions = questions();
    let mut correct = 0;
    for (state, index) in [
        ("I was charged twice for my order", 0),
        ("My parcel never arrived", 1),
        ("The app crashes when I log in", 2),
    ] {
        let probabilities = endpoint.ask(state, &test_questions[1..2]).await.unwrap();
        eprintln!("chat {state:?}: {probabilities:?}");
        if probabilities[0]
            .iter()
            .all(|value| *value <= probabilities[0][index])
        {
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
    let endpoint = endpoint("chat", &dev_config().endpoints["chat"]);
    let result = endpoint
        .ask("hi there, how are you?", &questions()[..1])
        .await;
    eprintln!("mass validation: {result:?}");
    assert!(result.is_ok());
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_rerank_relevance() {
    require(9933).await;
    let endpoint = endpoint("rerank", &dev_config().endpoints["rerank"]);
    let question: Question =
        serde_json::from_value(json!({"id":"q","type":"noul","instructions":"disk full"})).unwrap();
    for (state, relevant) in [("the disk is 98% full", true), ("a recipe for soup", false)] {
        let probabilities = endpoint
            .ask(state, std::slice::from_ref(&question))
            .await
            .unwrap();
        eprintln!("rerank {state:?}: {probabilities:?}");
        assert!(if relevant {
            probabilities[0][0] > 0.5
        } else {
            probabilities[0][0] < 0.5
        });
    }
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_embed() {
    require(9932).await;
    let config = dev_config();
    let embedder_config = config.embedder.as_ref().unwrap();
    let embedder = embed::Embedder::new(
        embedder_config,
        api_key(embedder_config.api_key_env.as_deref()),
    );
    let vectors = embedder
        .embed(&[
            "disk is full".into(),
            "storage at 98%".into(),
            "a recipe for soup".into(),
        ])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 3);
    for vector in &vectors {
        assert_eq!(vector.len(), 768);
    }
    let storage = embed::cosine(&vectors[0], &vectors[1]);
    let soup = embed::cosine(&vectors[0], &vectors[2]);
    eprintln!(
        "embedding dimensions: {}; disk/storage cosine: {storage}, disk/soup: {soup}",
        vectors[0].len()
    );
    assert!(storage > soup);
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_triage_with_prefilter() {
    require(9931).await;
    require(9932).await;
    let (service, _directory) = calibration_service(dev_config()).await;
    let request: service::TriageRequest = serde_json::from_value(json!({"items":[
        "The disk on /var is 98% full.","Ceph OSD down, PGs degraded.","Database backups failing; no recent recovery snapshot.",
        "A recipe for tomato soup.","The football match ended in a draw.","Good morning everyone!","The concert starts at eight.","My parcel arrived today.","The library opens tomorrow.","Sunny weather for a picnic."
    ],"question":{"id":"q","type":"noul","instructions":"Is storage or data at risk?"},"top_k":5})).unwrap();
    let result = service.triage(request).await.unwrap();
    eprintln!("prefilter triage: {result:?}");
    assert_eq!(
        result
            .kept
            .iter()
            .map(|item| item.index)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(result.warning.is_none());
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_doctor() {
    for port in [9931, 9932, 9933, 9934] {
        require(port).await;
    }
    let (service, _directory) = calibration_service(dev_config()).await;
    let report = service.doctor().await;
    eprintln!("doctor: {report:?}");
    for endpoint in &report.endpoints {
        assert!(endpoint.error.is_none());
        if let Some(probe) = &endpoint.probe {
            assert!(probe["error"].is_null());
        }
    }
    let d3 = report
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "d3")
        .unwrap();
    assert_eq!(d3.probe.as_ref().unwrap()["verdict"], "yes");
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_cache_skips_model() {
    require(9931).await;
    let mut config = dev_config();
    config.specs.insert(
        "greeting".into(),
        toml::from_str("type='noul'\ninstructions='Is this message a greeting?'").unwrap(),
    );
    let (service, _directory) = calibration_service(config).await;
    let request = spec_request("hi there, how are you?", "greeting");
    let first = service.ask(request.clone()).await.unwrap();
    let start = std::time::Instant::now();
    let second = service.ask(request).await.unwrap();
    let elapsed = start.elapsed();
    eprintln!("live cache: {second:?}, elapsed {elapsed:?}");
    assert!(second.results[0].cached);
    assert_eq!(second.results[0].id, first.results[0].id);
    assert!(elapsed < std::time::Duration::from_millis(50));
}

async fn calibration_service(config: Config) -> (Service, tempfile::TempDir) {
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();

    (Service::new(config, store), directory)
}

fn spec_request(state: &str, spec: &str) -> service::AskRequest {
    serde_json::from_value(json!({"state":state,"spec":spec})).unwrap()
}

fn fixture_rows(spec: &str) -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{spec}.jsonl"));
    let rows: Vec<_> = fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            (
                row["state"].as_str().unwrap().to_owned(),
                row["label"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(rows.len(), 60);
    assert_eq!(rows.iter().filter(|(_, label)| label == "yes").count(), 30);
    assert_eq!(rows.iter().filter(|(_, label)| label == "no").count(), 30);
    assert_eq!(
        rows.iter()
            .map(|(state, _)| state)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        60
    );

    rows
}

async fn fit_fixture(service: &Service, spec: &str) -> service::CalibrateResponse {
    for (index, (state, label)) in fixture_rows(spec).into_iter().enumerate() {
        if index % 3 == 0 {
            continue;
        }
        let result = service.ask(spec_request(&state, spec)).await.unwrap();
        assert_eq!(result.results.len(), 1);
        let decision = &result.results[0];
        assert_eq!(decision.question, spec);
        eprintln!(
            "{spec} fit {state:?}: label={label}, probabilities={:?}",
            decision.probabilities
        );
        service.label(decision.id, label).await.unwrap();
    }
    let result = service.calibrate(spec, Some(0.95), false).await.unwrap();
    eprintln!(
        "{spec} calibration: bands={:?}, report={:?}, stored={}",
        result.bands, result.report, result.stored
    );
    assert_eq!(result.report.examples, 40);
    assert!(result.stored);
    if result.report.precision < 0.95 {
        assert!(
            matches!(result.bands,verdict::Bands::Noul {yes_at,no_at} if yes_at>1.0 && no_at<0.0)
        );
        assert!(!result.report.warnings.is_empty());
        eprintln!("{spec}: never-deciding bands with warning");
    } else {
        eprintln!("{spec}: fitted precision target achieved");
    }

    result
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_calibrate_holdout() {
    require(9931).await;
    let (service, _directory) = calibration_service(dev_config()).await;
    for spec in ["greeting", "storage-risk"] {
        fit_fixture(&service, spec).await;
        let mut decided = 0_u32;
        let mut correct = 0_u32;
        for (index, (state, label)) in fixture_rows(spec).into_iter().enumerate() {
            if index % 3 != 0 {
                continue;
            }
            let result = service.ask(spec_request(&state, spec)).await.unwrap();
            let result = &result.results[0];
            assert!(result.calibrated);
            eprintln!("{spec} holdout {state:?}: label={label}, result={result:?}");
            if result.verdict != "unsure" {
                decided += 1;
                if result.verdict == label {
                    correct += 1;
                }
            }
        }
        let coverage = f64::from(decided) / 20.0;
        if decided > 0 {
            let precision = f64::from(correct) / f64::from(decided);
            eprintln!(
                "{spec} holdout: precision={precision}, coverage={coverage}, decided={decided}/20, correct={correct}"
            );
            assert!(precision >= 0.85, "{spec}: holdout precision {precision}");
        } else {
            eprintln!(
                "{spec} holdout: precision=undefined (no decisions), coverage={coverage}, decided=0/20"
            );
        }
    }
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_question_edit_invalidates() {
    require(9931).await;
    let (service, directory) = calibration_service(dev_config()).await;
    fit_fixture(&service, "greeting").await;
    let original = fs::read_to_string(dev_config_path()).unwrap();
    let edited = original.replace(
        "Is this message a greeting?",
        "Is this message a greeting exactly?",
    );
    let path = directory.path().join("edited.toml");
    fs::write(&path, edited).unwrap();
    let changed = Service::new(
        Config::load(&path).unwrap(),
        Store::open(&directory.path().join("db.sqlite"))
            .await
            .unwrap(),
    );
    let result = changed
        .ask(spec_request("hi there", "greeting"))
        .await
        .unwrap();
    assert!(!result.results[0].calibrated);
    let calibration = changed
        .calibrate("greeting", Some(0.95), false)
        .await
        .unwrap();
    eprintln!("edited question: {result:?}; calibration={calibration:?}");
    assert_eq!(calibration.report.examples, 0);
}

fn mcp_command(config: &Path, database_path: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
    command
        .arg("mcp")
        .arg("--config")
        .arg(config)
        .arg("--db")
        .arg(database_path)
        .kill_on_drop(true);

    command
}

struct LiveServer {
    child: std::process::Child,
    url: String,
    stderr: Option<std::thread::JoinHandle<String>>,
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            eprintln!("HTTP server stderr: {}", stderr.join().unwrap());
        }
    }
}

fn live_server(config: &Path, database_path: &Path, key: Option<&str>) -> LiveServer {
    let mut command = Command::new(env!("CARGO_BIN_EXE_8ball"));
    command
        .arg("serve")
        .arg("--config")
        .arg(config)
        .arg("--db")
        .arg(database_path)
        .args(["--listen", "127.0.0.1:0"])
        .stderr(std::process::Stdio::piped());
    if let Some(key) = key {
        command.args(["--api-key", key]);
    }
    let mut server = LiveServer {
        child: command.spawn().unwrap(),
        url: String::new(),
        stderr: None,
    };
    let mut errors = String::new();
    let mut reader = std::io::BufReader::new(server.child.stderr.take().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if let Some((_, address)) = line.split_once("listening on ") {
            server.url = format!("http://{}", address.trim());
            server.stderr = Some(std::thread::spawn(move || {
                let mut logs = String::new();
                let _ = reader.read_to_string(&mut logs);
                logs
            }));

            return server;
        }
        errors.push_str(&line);
    }

    panic!("HTTP server did not start: {errors}");
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_mcp_handler_ask() {
    require(9931).await;
    let (service, _directory) = calibration_service(dev_config()).await;
    let (server_io, client_io) = tokio::io::duplex(16_384);
    tokio::spawn(async move {
        service::mcp::Mcp::new(std::sync::Arc::new(service))
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("decision_ask")
                .with_arguments(object!({"state":"hi there, how are you?","spec":"greeting"})),
        )
        .await
        .unwrap();
    eprintln!("MCP handler real response: {result:?}");
    assert_eq!(
        result.structured_content.unwrap()["results"][0]["verdict"],
        "yes"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_mcp_stdio_ask() {
    require(9931).await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let config = dev_config_path();
    let client = ()
        .serve(
            TokioChildProcess::new(mcp_command(&config, &directory.path().join("db.sqlite")))
                .unwrap(),
        )
        .await
        .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 5);
    let info = client.peer_info().unwrap();
    let instructions = info.instructions.as_deref().unwrap();
    assert!(instructions.contains("greeting") && instructions.contains("storage-risk"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("decision_ask")
                .with_arguments(object!({"state":"hi there, how are you?","spec":"greeting"})),
        )
        .await
        .unwrap();
    eprintln!("MCP stdio real response: {result:?}");
    assert_eq!(
        result.structured_content.unwrap()["results"][0]["verdict"],
        "yes"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_mcp_http_triage() {
    require(9931).await;
    require(9932).await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let config = dev_config_path();
    let server = live_server(&config, &directory.path().join("db.sqlite"), Some("k"));
    let transport = rmcp::transport::StreamableHttpClientTransport::from_config(
        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
            format!("{}/mcp", server.url),
        )
        .auth_header("k"),
    );
    let client = ().serve(transport).await.unwrap();
    let arguments = object!({"items":["The disk on /var is 98% full.","Ceph OSD down, PGs degraded.","Database backups failing; no recent recovery snapshot.","A recipe for tomato soup.","The football match ended in a draw.","Good morning everyone!","The concert starts at eight.","My parcel arrived today.","The library opens tomorrow.","Sunny weather for a picnic."],"question":{"id":"q","type":"noul","instructions":"Is storage or data at risk?"},"top_k":5});
    let result = client
        .call_tool(CallToolRequestParams::new("decision_triage").with_arguments(arguments))
        .await
        .unwrap();
    eprintln!("MCP HTTP real triage: {result:?}");
    let body = result.structured_content.unwrap();
    assert_eq!(
        body["kept"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
#[ignore = "needs dev/models.sh up"]
async fn live_mcp_failure_is_fail_open() {
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let config = directory.path().join("dead.toml");
    fs::write(
        &config,
        "default='d3'\n[endpoints.d3]\nurl='http://127.0.0.1:9'\nkind='systemone'",
    )
    .unwrap();
    let client = ()
        .serve(
            TokioChildProcess::new(mcp_command(&config, &directory.path().join("db.sqlite")))
                .unwrap(),
        )
        .await
        .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("decision_ask").with_arguments(
                object!({"state":"hi","questions":[{"id":"q","type":"noul","instructions":"Greeting?"}]}),
            ),
        )
        .await
        .unwrap();
    eprintln!("MCP real transport fail-open: {result:?}");
    assert_eq!(result.is_error, Some(true));
    assert!(
        result
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .any(|text| text.text.contains("Fail open"))
    );
    client.cancel().await.unwrap();
}
