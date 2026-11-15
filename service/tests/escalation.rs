use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sqlx::Connection;
use wiremock::{MockServer, matchers};

use domain::{config::BackendKind, verdict};
use service::{Decision, TriageResponse};

#[expect(
    dead_code,
    reason = "these tests call the service directly, without `serve`"
)]
mod common;

struct App {
    service: std::sync::Arc<service::Service>,
    store: store::Store,
    d3: MockServer,
    big: MockServer,
    directory: tempfile::TempDir,
}

/// `d3` answers noul by state (`yes-*` 0.97, `no-*` 0.03, else 0.5) and choices with `a`. `big` reads the prompt:
/// `maybe-yes` yes 0.95, `maybe-no` no 0.95, `maybe-fail` a 500, `maybe-slow` yes after 30 s, else 0.5.
async fn fixture(per_minute: u32, learning: &str) -> App {
    let d3 = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let state = body["state"].as_str().unwrap();
            let noul = if state.starts_with("yes-") {
                0.97
            } else if state.starts_with("no-") {
                0.03
            } else {
                0.5
            };
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = if question["type"] == "choice" {
                        json!({"type":"choice","probabilities":{"a":0.99,"b":0.01}})
                    } else {
                        json!({"type":"noul","noul":noul})
                    };

                    (id.clone(), answer)
                })
                .collect();

            wiremock::ResponseTemplate::new(200).set_body_json(json!({"answers":answers}))
        })
        .mount(&d3)
        .await;
    let big = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let prompt = body["messages"][1]["content"].as_str().unwrap();
            if prompt.contains("maybe-fail") {
                return wiremock::ResponseTemplate::new(500);
            }
            let p_yes: f64 = if prompt.contains("maybe-yes") || prompt.contains("maybe-slow") {
                0.95
            } else if prompt.contains("maybe-no") {
                0.05
            } else {
                0.5
            };
            let response = wiremock::ResponseTemplate::new(200).set_body_json(
                json!({"choices":[{"logprobs":{"content":[{"top_logprobs":[
                    {"token":"yes","logprob":p_yes.ln()},{"token":"no","logprob":(1.0 - p_yes).ln()}
                ]}]}}]}),
            );
            if prompt.contains("maybe-slow") {
                response.set_delay(Duration::from_secs(30))
            } else {
                response
            }
        })
        .mount(&big)
        .await;
    // `big` may take 60 s per call, so only the ask deadline bounds the slow escalation.
    let config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{d3}'\nkind='systemone'\n\
         [endpoints.big]\nurl='{big}'\nkind='logprob'\nmodel='big'\ntimeout_secs=60\n\
         [escalation]\nendpoint='big'\nper_minute={per_minute}\n\
         [specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n\
         [specs.quiet]\ntype='noul'\ninstructions='Quiet?'\nescalate=false\n\
         [learning]\nexplore=0.0\n{learning}",
        d3 = d3.uri(),
        big = big.uri(),
    ))
    .unwrap();
    let (service, store, directory) = common::service(config).await;

    App {
        service,
        store,
        d3,
        big,
        directory,
    }
}

async fn ask_all(app: &App, request: Value) -> Vec<Decision> {
    let request: service::AskRequest = serde_json::from_value(request).unwrap();
    app.service.ask(request).await.unwrap().results
}

async fn ask(app: &App, request: Value) -> Decision {
    ask_all(app, request).await.remove(0)
}

async fn greeting(app: &App, state: &str) -> Decision {
    ask(app, json!({"spec":"greeting","state":state})).await
}

async fn triage(app: &App, request: Value) -> TriageResponse {
    let request: service::TriageRequest = serde_json::from_value(request).unwrap();
    app.service.triage(request).await.unwrap()
}

/// Model calls only: the first request also fetches endpoint info.
async fn requests(mock: &MockServer) -> Vec<Value> {
    mock.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

/// Stored rows as `(id, endpoint, state_hash)`, oldest first.
async fn rows(app: &App) -> Vec<(i64, String, String)> {
    let path = app.directory.path().join("db.sqlite");
    let mut connection = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();

    sqlx::query_as("SELECT id,endpoint,state_hash FROM decisions ORDER BY id")
        .fetch_all(&mut connection)
        .await
        .unwrap()
}

fn indices(items: &[service::TriageItem]) -> Vec<usize> {
    items.iter().map(|item| item.index).collect()
}

#[tokio::test]
async fn unsure_answer_is_escalated_and_both_rows_are_stored() {
    let app = fixture(30, "").await;
    let decision = greeting(&app, "maybe-yes-1").await;
    assert_eq!(decision.verdict, "yes");
    assert!(decision.escalated && !decision.cached && decision.report);
    let rows = rows(&app).await;
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].1.as_str(), rows[1].1.as_str()), ("d3", "big"));
    assert_eq!(rows[0].2, rows[1].2);
    assert_eq!(decision.id, rows[1].0);
}

#[tokio::test]
async fn decided_answer_is_not_escalated() {
    let app = fixture(30, "").await;
    let decision = greeting(&app, "yes-1").await;
    assert!(!decision.escalated);
    assert!(requests(&app.big).await.is_empty());
}

#[tokio::test]
async fn escalation_error_keeps_the_primary_unsure() {
    let app = fixture(30, "").await;
    let decision = greeting(&app, "maybe-fail-1").await;
    assert_eq!(decision.verdict, "unsure");
    assert!(!decision.escalated && decision.report);
    let rows = rows(&app).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "d3");
}

#[tokio::test]
async fn budget_caps_escalations() {
    let app = fixture(1, "").await;
    assert!(greeting(&app, "maybe-yes-1").await.escalated);
    let capped = greeting(&app, "maybe-yes-2").await;
    assert_eq!(capped.verdict, "unsure");
    assert!(!capped.escalated);
}

#[tokio::test]
async fn request_and_spec_switches() {
    let app = fixture(30, "").await;
    let off = ask(
        &app,
        json!({"spec":"greeting","state":"maybe-yes-1","escalate":false}),
    )
    .await;
    assert!(!off.escalated);
    let quiet = ask(&app, json!({"spec":"quiet","state":"maybe-yes-1"})).await;
    assert!(!quiet.escalated);
    let on = ask(
        &app,
        json!({"spec":"quiet","state":"maybe-yes-2","escalate":true}),
    )
    .await;
    assert!(on.escalated);
}

#[tokio::test]
async fn repeat_is_cached_on_both_endpoints() {
    let app = fixture(30, "").await;
    greeting(&app, "maybe-yes-1").await;
    let repeat = greeting(&app, "maybe-yes-1").await;
    assert!(repeat.cached && repeat.escalated);
    assert_eq!(requests(&app.d3).await.len(), 1);
    assert_eq!(requests(&app.big).await.len(), 1);
}

#[tokio::test]
async fn ad_hoc_questions_escalate() {
    let app = fixture(30, "").await;
    let decision = ask(
        &app,
        json!({"state":"maybe-yes-1","questions":[{"id":"q","type":"noul","instructions":"Official?"}]}),
    )
    .await;
    assert_eq!(decision.verdict, "yes");
    assert!(decision.escalated);
}

#[tokio::test]
async fn only_unsure_questions_of_an_ask_are_escalated() {
    let app = fixture(30, "").await;
    let results = ask_all(
        &app,
        json!({"state":"maybe-yes-1","questions":[
            {"id":"q","type":"noul","instructions":"Official?"},
            {"id":"c","type":"choice","instructions":"Bucket?","criteria":{"a":"","b":""}}
        ]}),
    )
    .await;
    assert_eq!(results[0].verdict, "yes");
    assert!(results[0].escalated);
    assert_eq!(results[1].verdict, "a");
    assert!(!results[1].escalated);
    assert_eq!(requests(&app.big).await.len(), 1);
}

#[tokio::test]
async fn triage_escalates_unsure_items() {
    let app = fixture(30, "").await;
    let response = triage(
        &app,
        json!({"spec":"greeting","items":["yes-1","maybe-yes-1","maybe-no-1","no-1"]}),
    )
    .await;
    assert_eq!(indices(&response.kept), [0, 1]);
    assert!(response.kept[1].decision.escalated);
    assert_eq!(indices(&response.rejected), [2, 3]);
    assert!(response.unsure.is_empty());
}

#[tokio::test]
async fn escalation_outcomes_are_counted() {
    let app = fixture(30, "").await;
    greeting(&app, "maybe-yes-1").await;
    greeting(&app, "maybe-fail-1").await;
    let metrics = app.service.metrics();
    assert!(
        metrics.contains(r#"eightball_escalations_total{endpoint="big",outcome="decided"} 1"#),
        "{metrics}"
    );
    assert!(
        metrics.contains(r#"eightball_escalations_total{endpoint="big",outcome="error"} 1"#),
        "{metrics}"
    );
    assert!(
        metrics
            .lines()
            .any(|line| line.starts_with("eightball_decisions_total")
                && line.contains(r#"endpoint="big""#)),
        "{metrics}"
    );
}

#[tokio::test]
async fn ask_deadline_keeps_the_primary_unsure() {
    let app = fixture(30, "").await;
    let started = Instant::now();
    let decision = greeting(&app, "maybe-slow-1").await;
    assert!(started.elapsed() < Duration::from_secs(25));
    assert_eq!(decision.verdict, "unsure");
    assert!(!decision.escalated && decision.report);
}

#[tokio::test]
async fn labelling_escalated_decisions_refits_both_endpoints() {
    let app = fixture(30, "recalibrate_after = 25").await;
    // 24 labels straight in the store skip the refit; the 25th, through the service, runs it.
    for i in 1..=25 {
        let decision = greeting(&app, &format!("maybe-yes-{i}")).await;
        assert!(decision.escalated);
        if i < 25 {
            app.store.set_label(decision.id, "yes").await.unwrap();
        } else {
            app.service.label(decision.id, "yes".into()).await.unwrap();
        }
    }
    let question: domain::question::Question =
        serde_json::from_value(json!({"id":"greeting","type":"noul","instructions":"Greeting?"}))
            .unwrap();
    let qhash = verdict::question_hash(&question);
    for calibration_id in [
        verdict::calibration_id(&qhash, BackendKind::Systemone, &app.d3.uri()),
        verdict::calibration_id(&qhash, BackendKind::Logprob, "big"),
    ] {
        assert!(
            app.store
                .calibration(&calibration_id, Some("greeting"))
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn review_shows_an_escalated_case_once() {
    let app = fixture(30, "").await;
    let decision = greeting(&app, "maybe-yes-1").await;
    let review = app
        .service
        .review(serde_json::from_value(json!({"spec":"greeting"})).unwrap())
        .await
        .unwrap();
    assert_eq!(review.decisions.len(), 1);
    assert_eq!(review.decisions[0].id, decision.id);
}
