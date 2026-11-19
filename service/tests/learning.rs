use serde_json::{Value, json};
use wiremock::MockServer;

use domain::verdict;
use service::{Decision, TriageResponse};

use common::{indices, label_under_old_model, requests};

mod common;

struct App {
    service: std::sync::Arc<service::Service>,
    store: store::Store,
    mock: MockServer,
    directory: tempfile::TempDir,
}

/// `common::systemone` as endpoint `d3`.
async fn fixture(learning: &str) -> App {
    let (service, store, mock, directory) = common::d3(&format!(
        "[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.bucket]\ntype='choice'\ninstructions='Bucket?'\ncriteria={{a='',b=''}}\n[learning]\n{learning}"
    ))
    .await;

    App {
        service,
        store,
        mock,
        directory,
    }
}

async fn ask(app: &App, request: Value) -> Decision {
    let request: service::AskRequest = serde_json::from_value(request).unwrap();
    app.service.ask(request).await.unwrap().results.remove(0)
}

async fn spec(app: &App, state: &str) -> Decision {
    ask(app, json!({"spec":"greeting","state":state})).await
}

/// The stored `greeting` fit on `d3`.
async fn fit(app: &App) -> Option<(verdict::Bands, verdict::Report)> {
    common::greeting_fit(
        &app.store,
        domain::config::BackendKind::Systemone,
        &app.mock.uri(),
    )
    .await
}

async fn triage(app: &App, request: Value) -> TriageResponse {
    let request: service::TriageRequest = serde_json::from_value(request).unwrap();
    app.service.triage(request).await.unwrap()
}

#[tokio::test]
async fn ask_reports_worked_and_unsure_decisions() {
    let app = fixture("explore = 0.0").await;
    let yes = spec(&app, "yes-1").await;
    let maybe = spec(&app, "maybe-1").await;
    let no = spec(&app, "no-1").await;
    let bucket = ask(&app, json!({"spec":"bucket","state":"yes-1"})).await;
    assert_eq!(
        [&yes, &maybe, &no, &bucket].map(|decision| decision.verdict.as_str()),
        ["yes", "unsure", "no", "a"]
    );
    assert!(yes.report && maybe.report);
    assert!(!no.report && !bucket.report);
    assert!(!no.explored && !yes.remembered);
    let adhoc = ask(
        &app,
        json!({"state":"maybe-1","questions":[{"id":"q","type":"noul","instructions":"Urgent?"}]}),
    )
    .await;
    assert!(adhoc.report, "a fresh ad-hoc question stores its text");

    let app = fixture("report = false").await;
    for state in ["yes-1", "maybe-1", "no-1"] {
        let decision = spec(&app, state).await;
        assert!(!decision.report && !decision.explored, "{state}");
    }
    let bucket = ask(&app, json!({"spec":"bucket","state":"yes-1"})).await;
    assert!(!bucket.report);
}

#[tokio::test]
async fn triage_reports_kept_and_unsure_but_never_rejected() {
    let app = fixture("explore = 0.0").await;
    let response = triage(
        &app,
        json!({"spec":"greeting","keep":"no","items":["no-1","no-2","yes-1","maybe-1"]}),
    )
    .await;
    assert_eq!(indices(&response.kept), [0, 1]);
    assert_eq!(indices(&response.rejected), [2]);
    assert_eq!(indices(&response.unsure), [3]);
    assert!(response.kept.iter().all(|item| item.decision.report));
    assert!(!response.rejected[0].decision.report);
    assert!(response.unsure[0].decision.report);
}

#[tokio::test]
async fn remembered_label_answers_without_the_model() {
    let app = fixture("").await;
    let labelled = label_under_old_model(&app.store, "maybe-1", "yes").await;
    let before = requests(&app.mock).await.len();
    let decision = spec(&app, "maybe-1").await;
    assert_eq!(decision.verdict, "yes");
    assert_eq!(decision.id, labelled);
    assert!(decision.remembered && decision.cached);
    assert!(!decision.report && !decision.explored);
    assert_eq!(requests(&app.mock).await.len(), before);
}

#[tokio::test]
async fn exploration_returns_skipped_answers_as_unsure() {
    let app = fixture("explore = 1.0").await;
    for _ in 0..2 {
        let decision = spec(&app, "no-1").await;
        assert_eq!(decision.verdict, "unsure");
        assert!(decision.explored && decision.report);
    }
    let yes = spec(&app, "yes-1").await;
    assert_eq!(yes.verdict, "yes");
    assert!(!yes.explored);
    let response = triage(
        &app,
        json!({"spec":"greeting","keep":"yes","items":["no-1","no-2","yes-1"]}),
    )
    .await;
    assert_eq!(indices(&response.unsure), [0, 1]);
    assert_eq!(indices(&response.kept), [2]);
    assert!(response.rejected.is_empty());
    assert!(response.unsure.iter().all(|item| item.decision.explored));

    let app = fixture("explore = 1.0\nreport = false").await;
    let decision = spec(&app, "no-1").await;
    assert_eq!(decision.verdict, "no");
    assert!(!decision.explored && !decision.report);
}

#[tokio::test]
async fn learn_false_returns_the_model_alone() {
    let app = fixture("explore = 1.0").await;
    label_under_old_model(&app.store, "maybe-1", "yes").await;
    let maybe = ask(
        &app,
        json!({"spec":"greeting","state":"maybe-1","learn":false}),
    )
    .await;
    assert_eq!(maybe.verdict, "unsure");
    assert!(!maybe.remembered && !maybe.explored && !maybe.report);
    let no = ask(
        &app,
        json!({"spec":"greeting","state":"no-1","learn":false}),
    )
    .await;
    assert_eq!(no.verdict, "no");
    assert!(!no.explored && !no.report);
    // Over REST, as `8ball evaluate` sends it.
    let url = common::serve(app.service.clone(), None).await;
    let response: TriageResponse = reqwest::Client::new()
        .post(format!("{url}/v1/triage"))
        .json(&json!({"spec":"greeting","items":["no-1","maybe-1"],"learn":false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(indices(&response.rejected), [0]);
    assert_eq!(indices(&response.unsure), [1]);
    assert!(!response.unsure[0].decision.remembered);
    assert!(!response.unsure[0].decision.report);
}

#[tokio::test]
async fn labels_refit_at_every_multiple_of_recalibrate_after() {
    let app = fixture("recalibrate_after = 25").await;
    for i in 1..=24 {
        let id = spec(&app, &format!("yes-{i}")).await.id;
        app.service.label(id, "yes".into()).await.unwrap();
    }
    assert!(fit(&app).await.is_none());
    assert!(!spec(&app, "maybe-1").await.calibrated);

    let id = spec(&app, "yes-25").await.id;
    let before = requests(&app.mock).await.len();
    app.service.label(id, "yes".into()).await.unwrap();
    assert_eq!(
        requests(&app.mock).await.len(),
        before,
        "a refit uses cached answers only"
    );
    let (bands, report) = fit(&app).await.unwrap();
    assert_eq!(
        bands,
        verdict::Bands::Noul {
            yes_at: 0.97,
            no_at: 0.15
        }
    );
    assert_eq!(report.examples, 25);
    assert!(spec(&app, "maybe-2").await.calibrated);
}

#[tokio::test]
async fn a_missed_refit_runs_at_the_next_label() {
    let app = fixture("recalibrate_after = 20").await;
    // Labels set straight in the store skip the refit, as a failed probe or a busy refit would.
    for i in 1..=21 {
        let id = spec(&app, &format!("yes-{i}")).await.id;
        app.store.set_label(id, "yes").await.unwrap();
    }
    let id = spec(&app, "yes-22").await.id;
    app.service.label(id, "yes".into()).await.unwrap();
    let (_, report) = fit(&app).await.unwrap();
    assert_eq!(report.examples, 22);
}

#[tokio::test]
async fn calibrate_reasks_the_model_instead_of_remembering() {
    let app = fixture("").await;
    label_under_old_model(&app.store, "yes-old", "yes").await;
    app.service.calibrate("greeting", None, true).await.unwrap();
    let asked = requests(&app.mock).await;
    assert!(asked.iter().any(|body| body["state"] == "yes-old"));
}

#[tokio::test]
async fn ad_hoc_questions_learn_by_their_stored_text() {
    let urgent = json!({"id":"q","type":"noul","instructions":"Urgent?"});
    let app = fixture("").await;
    let id = ask(&app, json!({"state":"maybe-1","questions":[urgent]}))
        .await
        .id;
    assert!(app.service.label(id, "maybe".into()).await.is_err());
    assert_eq!(
        app.service.label(id, "true".into()).await.unwrap().label,
        "yes"
    );
    let questions = app
        .service
        .questions(service::QuestionsRequest::default())
        .await
        .unwrap()
        .questions;
    assert!(questions[0].spec.is_none() && questions[0].labelable);

    let app = fixture("ad_hoc = false").await;
    let id = ask(&app, json!({"state":"maybe-1","questions":[urgent]}))
        .await
        .id;
    let error = app.service.label(id, "yes".into()).await.unwrap_err();
    assert!(error.to_string().contains("ad-hoc question"), "{error}");

    let app = fixture("").await;
    let id = ask(&app, json!({"state":"maybe-1","questions":[urgent]}))
        .await
        .id;
    let mut connection = common::connection(&app.directory).await;
    sqlx::query("DELETE FROM questions")
        .execute(&mut connection)
        .await
        .unwrap();
    let error = app.service.label(id, "yes".into()).await.unwrap_err();
    assert!(
        error.to_string().contains("question text unknown"),
        "{error}"
    );
}

#[tokio::test]
async fn triage_lists_duplicates_under_their_first_decision() {
    let app = fixture("explore = 0.0").await;
    let response = triage(
        &app,
        json!({"spec":"greeting","items":["yes-1","no-1","yes-1","maybe-1","no-1"]}),
    )
    .await;
    assert_eq!(indices(&response.kept), [0, 2]);
    assert_eq!(indices(&response.rejected), [1, 4]);
    assert_eq!(indices(&response.unsure), [3]);
    assert_eq!(response.dropped, 2);
    assert_eq!(response.kept[0].decision.id, response.kept[1].decision.id);
    assert_eq!(requests(&app.mock).await.len(), 3);
}

#[tokio::test]
async fn a_refit_that_keeps_the_bands_stores_its_report() {
    let app = fixture("recalibrate_after = 20").await;
    // Labels set straight in the store skip the refit; the 20th and the 40th, through the service, run it. Both
    // fits give the same bands.
    for i in 1..=40 {
        let id = spec(&app, &format!("yes-{i}")).await.id;
        if i % 20 == 0 {
            app.service.label(id, "yes".into()).await.unwrap();
        } else {
            app.store.set_label(id, "yes").await.unwrap();
        }
    }
    let (_, report) = fit(&app).await.unwrap();
    assert_eq!(report.examples, 40);
}
