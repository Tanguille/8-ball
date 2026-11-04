use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use eightball::evaluate::{Observation, Report};

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/triage"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let item = body["items"][0].as_str().unwrap();
            if item == "error" {
                return ResponseTemplate::new(502).set_body_json(json!({"error":"model unavailable"}));
            }
            if item == "malformed" {
                return ResponseTemplate::new(200).set_body_json(json!({"kept":[],"unsure":[],"rejected":[],"dropped":0,"prefiltered":[]}));
            }
            let (verdict, field) = match item {
                "skip" => ("no", "rejected"),
                "unsure" => ("unsure", "unsure"),
                _ => ("yes", "kept"),
            };
            let mut response = json!({"kept":[],"unsure":[],"rejected":[],"prefiltered":[],"dropped":usize::from(item == "skip")});
            response[field] = json!([{"index":0,"decision":{
                "id":1,"question":"relevant","type":"noul","verdict":verdict,
                "confidence":0.98,"probabilities":{"yes":0.98,"no":0.02},
                "calibrated":false,"cached":item == "cached"
            }}]);
            let response = ResponseTemplate::new(200).set_body_json(response);

            if item == "slow" { response.set_delay(std::time::Duration::from_secs(10)) } else { response }
        })
        .mount(&server).await;

    server
}

async fn evaluate(server: &MockServer, cases: &str, extra: &[&str]) -> std::process::Output {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "evaluate",
            "--spec",
            "relevant",
            "--url",
            &server.uri(),
            "--api-key",
            "test-key",
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(cases.as_bytes())
        .await
        .unwrap();

    child.wait_with_output().await.unwrap()
}

fn cases(rows: &[(&str, bool)]) -> String {
    rows.iter().enumerate().map(|(index, (item, expected))| {
        json!({"id":index.to_string(),"item":item,"context":format!("snapshot {index}"),"expected_keep":expected}).to_string() + "\n"
    }).collect()
}

#[tokio::test]
async fn replay_reports_misses_abstentions_errors_and_latency_without_sending_labels() {
    let server = server().await;
    let output = evaluate(
        &server,
        &cases(&[
            ("cached", true),
            ("skip", false),
            ("skip", true),
            ("keep", false),
            ("unsure", true),
            ("error", true),
        ]),
        &["--endpoint", "candidate"],
    )
    .await;
    assert_eq!(
        output.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value =
        serde_json::from_slice(&output.stdout).expect("evaluation JSON even on backend errors");
    let report = &body["report"];
    assert_eq!(report["total"], 6);
    assert_eq!(report["decided"], 4);
    assert_eq!(report["missed_useful"], 1);
    assert_eq!(report["unnecessary_keeps"], 1);
    assert_eq!(report["unsure"], 1);
    assert_eq!(report["errors"], 1);
    assert_eq!(report["would_skip"], 2);
    assert_eq!(report["skip_precision"], 0.5);
    assert_eq!(report["useful_recall"], 0.75);
    assert_eq!(report["cached"], 1);
    assert_eq!(report["uncached_latency"]["samples"], 4);
    assert_eq!(report["cached_latency"]["samples"], 1);
    assert_eq!(body["observations"][5]["would_keep"], true);
    assert!(
        body["observations"][5]["error"]
            .as_str()
            .unwrap()
            .contains("unavailable")
    );
    assert_eq!(body["spec"], "relevant");
    assert_eq!(body["endpoint"], "candidate");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 6);
    for request in requests {
        assert_eq!(request.url.path(), "/v1/triage");
        assert_eq!(
            request.headers.get("authorization").unwrap(),
            "Bearer test-key"
        );
        let sent: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(sent["spec"], "relevant");
        assert_eq!(sent["endpoint"], "candidate");
        assert_eq!(sent["keep"], "yes");
        assert!(sent["context"].as_str().unwrap().starts_with("snapshot "));
        assert!(sent.get("expected_keep").is_none());
    }
}

#[test]
fn latency_quantiles_separate_cached_requests_and_exclude_errors() {
    let decision: eightball::service::Decision = serde_json::from_value(json!({
        "id":1,"question":"relevant","type":"noul","verdict":"yes",
        "confidence":0.98,"probabilities":{"yes":0.98,"no":0.02},
        "calibrated":false,"cached":false
    }))
    .unwrap();
    let mut observations: Vec<_> = (1..=21)
        .rev()
        .map(|elapsed| Observation {
            id: elapsed.to_string(),
            expected_keep: true,
            would_keep: true,
            elapsed_ms: f64::from(elapsed),
            decision: Some(decision.clone()),
            error: None,
        })
        .collect();
    let even = Report::from_observations(&observations[1..]);
    assert_eq!(even.uncached_latency.samples, 20);
    assert_eq!(even.uncached_latency.p50_ms, Some(10.0));
    assert_eq!(even.uncached_latency.p95_ms, Some(19.0));
    assert_eq!(even.cached_latency.samples, 0);
    assert_eq!(even.cached_latency.p50_ms, None);
    assert_eq!(even.cached_latency.p95_ms, None);
    for elapsed in [200.0, 100.0] {
        let mut cached = decision.clone();
        cached.cached = true;
        observations.push(Observation {
            id: elapsed.to_string(),
            expected_keep: true,
            would_keep: true,
            elapsed_ms: elapsed,
            decision: Some(cached),
            error: None,
        });
    }
    observations.push(Observation {
        id: "failed".into(),
        expected_keep: true,
        would_keep: true,
        elapsed_ms: 99_999.0,
        decision: None,
        error: Some("request failed".into()),
    });
    let report = Report::from_observations(&observations);
    assert_eq!(report.errors, 1);
    assert_eq!(report.uncached_latency.samples, 21);
    assert_eq!(report.uncached_latency.p50_ms, Some(11.0));
    assert_eq!(report.uncached_latency.p95_ms, Some(20.0));
    assert_eq!(report.cached_latency.samples, 2);
    assert_eq!(report.cached_latency.p50_ms, Some(100.0));
    assert_eq!(report.cached_latency.p95_ms, Some(200.0));
}

#[tokio::test]
async fn validates_entire_dataset_before_any_requests() {
    let server = server().await;
    for input in [
        String::new(),
        "not json\n".to_string(),
        "{\"id\":\"a\",\"item\":\"one\",\"expected_keep\":true}\n{\"id\":\"a\",\"item\":\"two\",\"expected_keep\":false}".to_string(),
        "{\"id\":\"a\",\"item\":\"one\"}".to_string(),
        "{\"id\":\"\",\"item\":\"one\",\"expected_keep\":true}".to_string(),
        cases(&[("keep",true)]) + "not json\n",
    ] {
        let output = evaluate(&server, &input, &[]).await;
        assert_eq!(output.status.code(), Some(4), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stdout.is_empty());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn timeout_and_malformed_success_are_errors_that_keep_work() {
    let server = server().await;
    let output = evaluate(
        &server,
        &cases(&[("slow", true), ("malformed", false)]),
        &["--timeout-ms", "100"],
    )
    .await;
    assert_eq!(
        output.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["report"]["errors"], 2);
    assert_eq!(body["report"]["decided"], 0);
    assert!(body["report"]["skip_precision"].is_null());
    assert!(body["report"]["decision_accuracy"].is_null());
    assert!(body["report"]["uncached_latency"]["p95_ms"].is_null());
    for observation in body["observations"].as_array().unwrap() {
        assert_eq!(observation["would_keep"], true);
        assert!(observation["error"].is_string());
    }
}

#[tokio::test]
async fn clean_replay_exits_zero_and_does_not_claim_recall_without_positives() {
    let server = server().await;
    let output = evaluate(&server, &cases(&[("skip", false)]), &[]).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(body["report"]["useful_recall"].is_null());
    assert_eq!(body["report"]["skip_precision"], 1.0);
}
