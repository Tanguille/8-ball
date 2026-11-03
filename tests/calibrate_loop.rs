use eightball::{
    backend::Endpoint,
    config::Config,
    http::router,
    question::Question,
    service::{AskRequest, Service},
    store::{NewDecision, Store},
    verdict::{question_hash, short_hash},
};
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

struct App {
    service: Arc<Service>,
    model: MockServer,
    dir: tempfile::TempDir,
    url: String,
    task: tokio::task::JoinHandle<()>,
    config: Config,
    store: Store,
}
impl Drop for App {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn app() -> App {
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|r: &Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let state = body["state"].as_str().unwrap();
            let offset = f64::from(state.bytes().fold(0_u8, u8::wrapping_add) % 10) / 100.0;
            let p = if state.contains("hello") {
                0.90 + offset
            } else {
                0.01 + offset
            };
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, q)| {
                    let answer = match q["type"].as_str().unwrap() {
                        "noul" => json!({"type":"noul","noul":p}),
                        "choice" => json!({"type":"choice","probabilities":{"a":0.9,"b":0.1}}),
                        "score" => json!({"type":"score","probabilities":{"0":0.1,"1":0.9}}),
                        _ => unreachable!(),
                    };
                    (id.clone(), answer)
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&model)
        .await;
    let config:Config=toml::from_str(&format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.team]\ntype='choice'\ninstructions='Team?'\ncriteria={{a='',b=''}}\n[specs.urgency]\ntype='score'\ninstructions='Urgent?'\ncriteria=['low','high']",model.uri())).unwrap();
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, cfg)| (name.clone(), Endpoint::new(name, cfg, None).unwrap()))
        .collect();
    let service = Arc::new(Service::new(config.clone(), endpoints, store.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = service.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router(served)).await.unwrap();
    });
    App {
        service,
        model,
        dir,
        url,
        task,
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
fn req(spec: &str, state: &str) -> AskRequest {
    AskRequest {
        spec: Some(spec.into()),
        state: state.into(),
        questions: vec![],
        endpoint: None,
    }
}
async fn posts(app: &App) -> usize {
    app.model
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
        .count()
}
#[tokio::test]
async fn full_ask_label_calibrate_loop_reuses_cache_and_invalidates_edits() {
    let app = app().await;
    for i in 0..50 {
        let yes = i < 25;
        let state = if yes {
            format!("hello message {i}")
        } else {
            format!("invoice message {i}")
        };
        let decision = app
            .service
            .ask(req("greeting", &state))
            .await
            .unwrap()
            .results
            .remove(0);
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
        !app.service
            .ask(req("greeting", "hello dry run"))
            .await
            .unwrap()
            .results[0]
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
        app.service
            .ask(req("greeting", "hello new state"))
            .await
            .unwrap()
            .results[0]
            .calibrated
    );
    let mut config = app.config.clone();
    config.specs["greeting"].instructions.push_str(" clearly");
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, cfg)| (name.clone(), Endpoint::new(name, cfg, None).unwrap()))
        .collect();
    let changed = Service::new(
        config,
        endpoints,
        Store::open(&app.dir.path().join("db.sqlite")).unwrap(),
    );
    assert!(
        !changed
            .ask(req("greeting", "hello new state"))
            .await
            .unwrap()
            .results[0]
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
    let adhoc: AskRequest = serde_json::from_value(
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
        let id = app
            .service
            .ask(req(spec, "hello label"))
            .await
            .unwrap()
            .results[0]
            .id;
        let result = app.service.label(id, good.into()).await.unwrap();
        assert_eq!(result.label, if spec == "greeting" { "yes" } else { good });
        assert!(app.service.label(id, bad.into()).await.is_err());
    }
    let id = app
        .service
        .ask(req("greeting", "hello old"))
        .await
        .unwrap()
        .results[0]
        .id;
    let mut config = app.config.clone();
    config.specs["greeting"].instructions.push_str(" now");
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, cfg)| (name.clone(), Endpoint::new(name, cfg, None).unwrap()))
        .collect();
    let changed = Service::new(config, endpoints, app.store.clone());
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
    let id = app
        .service
        .ask(req("greeting", state))
        .await
        .unwrap()
        .results[0]
        .id;
    app.service.label(id, "yes".into()).await.unwrap();
    let q: Question =
        serde_json::from_value(json!({"id":"greeting","type":"noul","instructions":"Greeting?"}))
            .unwrap();
    let other = app
        .store
        .insert(NewDecision {
            spec: Some("greeting".into()),
            question_hash: question_hash(&q),
            calibration_id: "previous-model".into(),
            state_hash: short_hash(state),
            state: state.into(),
            answer: vec![0.9, 0.1],
            verdict: "yes".into(),
            endpoint: "d3".into(),
        })
        .await
        .unwrap();
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
            .any(|w| w.starts_with("conflicting labels for decision ids"))
    );
}

#[tokio::test]
async fn cli_supports_spec_labels_calibration_and_exit_codes() {
    let app = app().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
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
    let label = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["label", &id, "yes", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert!(label.status.success());
    let bad = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["label", &id, "maybe", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert_eq!(bad.status.code(), Some(4));
    let calibration = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
        .args(["calibrate", "greeting", "--dry-run", "--url", &app.url])
        .output()
        .await
        .unwrap();
    assert!(calibration.status.success());
    let report: Value = serde_json::from_slice(&calibration.stdout).unwrap();
    assert_eq!(report["stored"], false);
    let no = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"))
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
