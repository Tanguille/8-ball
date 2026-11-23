use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wiremock::{MockServer, matchers};

use domain::config::BackendKind;
use service::{Decision, TriageResponse};

use common::{indices, requests};

#[path = "utils/common.rs"]
mod common;

struct App {
    service: std::sync::Arc<service::Service>,
    store: store::Store,
    d3: MockServer,
    big: MockServer,
    directory: tempfile::TempDir,
    config: String,
}

/// `d3` is `common::systemone`. `big` reads the prompt:
/// `maybe-yes` yes 0.95, `maybe-no` no 0.95, `maybe-fail` a 500, `maybe-slow` yes after 5 s, past both
/// deadlines, which the fixture shortens to 1 s, else 0.5.
async fn fixture(per_minute: usize, learning: &str) -> App {
    let d3 = common::systemone().await;
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
                response.set_delay(Duration::from_secs(5))
            } else {
                response
            }
        })
        .mount(&big)
        .await;
    // `big` may take 120 s per call, so only the ask or triage deadline bounds the slow escalation.
    let config = format!(
        "default='d3'\n[endpoints.d3]\nurl='{d3}'\nkind='systemone'\n\
         [endpoints.big]\nurl='{big}'\nkind='logprob'\nmodel='big'\ntimeout_secs=120\n\
         [escalation]\nendpoint='big'\nper_minute={per_minute}\n\
         [specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n\
         [specs.quiet]\ntype='noul'\ninstructions='Quiet?'\nescalate=false\n\
         [learning]\nexplore=0.0\n{learning}",
        d3 = d3.uri(),
        big = big.uri(),
    );
    let (store, directory) = common::store().await;
    let service = service::Service::new(toml::from_str(&config).unwrap(), store.clone())
        .with_deadlines(Duration::from_secs(1), Duration::from_secs(1));

    App {
        service: std::sync::Arc::new(service),
        store,
        d3,
        big,
        directory,
        config,
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

/// Stored rows as `(id, endpoint, state_hash)`, oldest first.
async fn rows(app: &App) -> Vec<(i64, String, String)> {
    let mut connection = common::connection(&app.directory).await;

    sqlx::query_as("SELECT id,endpoint,state_hash FROM decisions ORDER BY id")
        .fetch_all(&mut connection)
        .await
        .unwrap()
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
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(decision.verdict, "unsure");
    assert!(!decision.escalated && decision.report);
    let metrics = app.service.metrics();
    assert!(
        metrics.contains(r#"eightball_backend_errors_total{endpoint="big"} 1"#),
        "{metrics}"
    );
    assert!(
        metrics.contains(r#"eightball_escalations_total{endpoint="big",outcome="error"} 1"#),
        "{metrics}"
    );
}

#[tokio::test]
async fn triage_deadline_keeps_the_primary_unsure() {
    let app = fixture(30, "").await;
    let response = triage(
        &app,
        json!({"spec":"greeting","items":["yes-1","maybe-slow-1"]}),
    )
    .await;
    assert_eq!(indices(&response.kept), [0]);
    assert_eq!(indices(&response.unsure), [1]);
    let decision = &response.unsure[0].decision;
    assert!(decision.error.is_none() && !decision.escalated && decision.report);
    assert!(response.warning.is_none(), "{:?}", response.warning);
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
    for (kind, fingerprint) in [
        (BackendKind::Systemone, app.d3.uri()),
        (BackendKind::Logprob, "big".into()),
    ] {
        assert!(
            common::greeting_fit(&app.store, kind, &fingerprint)
                .await
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

#[tokio::test]
async fn labelling_a_state_never_escalated_leaves_the_escalation_endpoint_alone() {
    let app = fixture(30, "recalibrate_after = 1").await;
    let decision = greeting(&app, "yes-1").await;
    assert!(!decision.escalated);
    app.service.label(decision.id, "yes".into()).await.unwrap();
    assert!(app.big.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn labelling_refits_the_endpoints_that_answered_after_escalation_is_turned_off() {
    let app = fixture(30, "recalibrate_after = 25").await;
    let mut ids = Vec::new();
    for i in 1..=25 {
        ids.push(greeting(&app, &format!("maybe-yes-{i}")).await.id);
    }
    for &id in &ids[..24] {
        app.store.set_label(id, "yes").await.unwrap();
    }
    let mut config: toml::Table = toml::from_str(&app.config).unwrap();
    config.remove("escalation");
    let service = service::Service::new(config.try_into().unwrap(), app.store.clone());
    service.label(ids[24], "yes".into()).await.unwrap();
    assert!(
        common::greeting_fit(&app.store, BackendKind::Systemone, &app.d3.uri())
            .await
            .is_some()
    );
}

#[tokio::test]
async fn escalation_endpoint_refits_at_multiples_of_its_own_labels() {
    let app = fixture(100, "recalibrate_after = 25").await;
    let fitted = || async {
        common::greeting_fit(&app.store, BackendKind::Logprob, "big")
            .await
            .map(|(_, report)| report.examples)
    };
    // Five labels the escalation endpoint never saw put the question's label count 5 ahead of its own. Two `no`
    // labels on states both models find unsure keep d3 from fitting its yes side at 0.5.
    for i in 1..=5 {
        let id = greeting(&app, &format!("yes-{i}")).await.id;
        app.store.set_label(id, "yes").await.unwrap();
    }
    let labels = (1..=2)
        .map(|i| (format!("maybe-{i}"), "no"))
        .chain((1..=23).map(|i| (format!("maybe-yes-{i}"), "yes")));
    for (seen, (state, label)) in (1..).zip(labels) {
        let decision = greeting(&app, &state).await;
        assert!(decision.escalated, "{state}");
        app.service.label(decision.id, label.into()).await.unwrap();
        if seen == 22 {
            assert_eq!(fitted().await, None, "22 of its own labels refit nothing");
        }
    }
    assert_eq!(fitted().await, Some(25));
}

#[tokio::test]
async fn cached_escalations_do_not_spend_the_budget() {
    let app = fixture(2, "").await;
    let items = json!({"spec":"greeting","items":["maybe-yes-1"]});
    for _ in 0..2 {
        let response = triage(&app, items.clone()).await;
        assert!(response.kept[0].decision.escalated);
    }
    assert_eq!(requests(&app.big).await.len(), 1);
    assert!(greeting(&app, "maybe-yes-2").await.escalated);
}

#[tokio::test]
async fn a_missed_triage_deadline_is_a_counted_backend_error() {
    let app = fixture(30, "").await;
    let response = triage(&app, json!({"spec":"greeting","items":["yes-1","slow-1"]})).await;
    let error = response.unsure[0].decision.error.as_deref().unwrap();
    assert!(error.contains("triage deadline"), "{error}");
    let metrics = app.service.metrics();
    assert!(
        metrics.contains(r#"eightball_backend_errors_total{endpoint="d3"} 1"#),
        "{metrics}"
    );
}
