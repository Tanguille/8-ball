use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::{MockServer, matchers};

use domain::{config::Config, verdict};
use service::Service;
use store::Store;

#[path = "../../service/tests/common.rs"]
mod common;

struct App {
    service: std::sync::Arc<Service>,
    model: MockServer,
    directory: tempfile::TempDir,
    url: String,
    config: Config,
    store: Store,
}
async fn app() -> App {
    let model = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let state = body["state"].as_str().unwrap();
            let offset = f64::from(state.bytes().fold(0_u8, u8::wrapping_add) % 10) / 100.0;
            let probabilities = if state.contains("hello") {
                0.90 + offset
            } else {
                0.01 + offset
            };
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = match question["type"].as_str().unwrap() {
                        "noul" => json!({"type":"noul","noul":probabilities}),
                        "choice" => json!({"type":"choice","probabilities":{"a":0.9,"b":0.1}}),
                        "score" => json!({"type":"score","probabilities":{"0":0.1,"1":0.9}}),
                        _ => unreachable!(),
                    };

                    (id.clone(), answer)
                })
                .collect();

            wiremock::ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&model)
        .await;
    let config: Config = toml::from_str(
        &format!(
            "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.team]\ntype='choice'\ninstructions='Team?'\ncriteria={{a='',b=''}}\n[specs.urgency]\ntype='score'\ninstructions='Urgent?'\ncriteria=['low','high']",
            model.uri(),
        ),
    )
    .unwrap();
    let (service, store, directory) = common::service(config.clone()).await;
    let url = common::serve(service.clone(), None).await;

    App {
        service,
        model,
        directory,
        url,
        config,
        store,
    }
}
async fn post(app: &App, path: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}{path}", app.url))
        .json(&body)
        .send()
        .await
        .unwrap()
}
async fn ask(service: &Service, spec: &str, state: &str) -> service::Decision {
    service
        .ask(serde_json::from_value(json!({"spec":spec,"state":state})).unwrap())
        .await
        .unwrap()
        .results
        .remove(0)
}
async fn posts(app: &App) -> usize {
    app.model
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
        .count()
}
#[tokio::test]
async fn full_ask_label_calibrate_loop_reuses_cache_and_invalidates_edits() {
    let app = app().await;
    for number in 0..50 {
        let yes = number < 25;
        let state = if yes {
            format!("hello message {number}")
        } else {
            format!("invoice message {number}")
        };
        let decision = ask(&app.service, "greeting", &state).await;
        assert_eq!(
            post(
                &app,
                "/v1/label",
                json!({"id":decision.id,"value":if yes {"yes"} else {"no"}})
            )
            .await
            .status(),
            200
        );
    }
    let before = posts(&app).await;
    let result = post(
        &app,
        "/v1/calibrate",
        json!({"spec":"greeting","precision":0.95,"dry_run":true}),
    )
    .await;
    assert_eq!(result.status(), 200);
    let dry: Value = result.json().await.unwrap();
    assert_eq!(dry["stored"], false);
    assert!(
        !ask(&app.service, "greeting", "hello dry run")
            .await
            .calibrated
    );
    let before_stored = posts(&app).await;
    assert_eq!(before_stored, before + 1);
    let result = post(
        &app,
        "/v1/calibrate",
        json!({"spec":"greeting","precision":0.95,"dry_run":false}),
    )
    .await;
    assert_eq!(result.status(), 200);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["stored"], true);
    assert!(result["report"]["coverage"].as_f64().unwrap() > 0.9);
    assert_eq!(posts(&app).await, before_stored);
    assert!(
        ask(&app.service, "greeting", "hello new state")
            .await
            .calibrated
    );
    let mut config = app.config.clone();
    config.specs["greeting"].instructions.push_str(" clearly");
    let changed = Service::new(
        config,
        Store::open(&app.directory.path().join("db.sqlite"))
            .await
            .unwrap(),
    );
    assert!(
        !ask(&changed, "greeting", "hello new state")
            .await
            .calibrated
    );
    assert_eq!(
        changed
            .calibrate("greeting", Some(0.95), false)
            .await
            .unwrap()
            .report
            .examples,
        0
    );
}
#[tokio::test]
async fn label_validation_rejects_unknown_adhoc_old_and_invalid_values() {
    let app = app().await;
    assert!(
        app.service
            .label(99_999, "yes".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("no decision")
    );
    let adhoc: service::AskRequest = serde_json::from_value(
        json!({"state":"hello","questions":[{"id":"q","type":"noul","instructions":"Ad hoc?"}]}),
    )
    .unwrap();
    let id = app.service.ask(adhoc).await.unwrap().results[0].id;
    assert!(
        app.service
            .label(id, "yes".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("ad-hoc question")
    );
    for (spec, good, bad) in [
        ("greeting", "true", "maybe"),
        ("team", "a", "c"),
        ("urgency", "1", "2"),
    ] {
        let id = ask(&app.service, spec, "hello label").await.id;
        let result = app.service.label(id, good.into()).await.unwrap();
        assert_eq!(result.label, if spec == "greeting" { "yes" } else { good });
        assert!(app.service.label(id, bad.into()).await.is_err());
    }
    let id = ask(&app.service, "greeting", "hello old").await.id;
    let mut config = app.config.clone();
    config.specs["greeting"].instructions.push_str(" now");
    let changed = Service::new(config, app.store.clone());
    assert!(
        changed
            .label(id, "yes".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("older version")
    );
}
#[tokio::test]
async fn conflicting_labels_are_excluded() {
    let app = app().await;
    let state = "hello conflict";
    let id = ask(&app.service, "greeting", state).await.id;
    app.service.label(id, "yes".into()).await.unwrap();
    let question: domain::question::Question =
        serde_json::from_value(json!({"id":"greeting","type":"noul","instructions":"Greeting?"}))
            .unwrap();
    let other = app
        .store
        .insert_many(vec![store::NewDecision {
            spec: Some("greeting".into()),
            question_hash: verdict::question_hash(&question),
            question: "{}".into(),
            calibration_id: "previous-model".into(),
            state_hash: verdict::short_hash(state),
            state: state.into(),
            answer: vec![0.9, 0.1],
            verdict: "yes".into(),
            endpoint: "d3".into(),
        }])
        .await
        .unwrap()[0];
    app.service.label(other, "no".into()).await.unwrap();
    let result = app
        .service
        .calibrate("greeting", Some(0.95), true)
        .await
        .unwrap();
    assert_eq!(result.report.examples, 0);
    assert!(
        result
            .report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("conflicting labels for decision ids"))
    );
}

#[tokio::test]
async fn cli_supports_spec_labels_calibration_and_exit_codes() {
    let app = app().await;
    let output = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "ask",
            "--spec",
            "greeting",
            "--state",
            "hello cli",
            "--url",
            &app.url,
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = result["results"][0]["id"].as_i64().unwrap().to_string();
    let label = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["label", &id, "yes", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert!(label.status.success());
    let bad = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["label", &id, "maybe", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert_eq!(bad.status.code(), Some(4));
    let calibration = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["calibrate", "greeting", "--dry-run", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert!(calibration.status.success());
    let report: Value = serde_json::from_slice(&calibration.stdout).unwrap();
    assert_eq!(report["stored"], false);
    let no = Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args([
            "ask",
            "--spec",
            "greeting",
            "--state",
            "invoice cli",
            "--exit-code",
            "--url",
            &app.url,
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(
        no.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&no.stderr)
    );
}
