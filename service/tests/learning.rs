use serde_json::{Value, json};
use sqlx::Connection;
use wiremock::{MockServer, matchers};

use domain::verdict;
use service::{Decision, TriageResponse};

mod common;

struct App {
    service: std::sync::Arc<service::Service>,
    store: store::Store,
    mock: MockServer,
    directory: tempfile::TempDir,
}

/// A systemone mock that answers by state: `yes-*` 0.97, `maybe-*` 0.5, `no-*` 0.03, and option `a` for choices.
async fn fixture(learning: &str) -> App {
    let mock = MockServer::start().await;
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
        .mount(&mock)
        .await;
    let config = toml::from_str(&format!(
        "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'\n[specs.bucket]\ntype='choice'\ninstructions='Bucket?'\ncriteria={{a='',b=''}}\n[learning]\n{learning}",
        mock.uri(),
    ))
    .unwrap();
    let (service, store, directory) = common::service(config).await;

    App {
        service,
        store,
        mock,
        directory,
    }
}

fn greeting() -> domain::question::Question {
    serde_json::from_value(json!({"id":"greeting","type":"noul","instructions":"Greeting?"}))
        .unwrap()
}

async fn ask(app: &App, request: Value) -> Decision {
    let request: service::AskRequest = serde_json::from_value(request).unwrap();
    app.service.ask(request).await.unwrap().results.remove(0)
}

async fn spec(app: &App, state: &str) -> Decision {
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

/// A labeled `greeting` decision under a model the service never uses, so only `remember` can find it.
async fn label_under_old_model(app: &App, state: &str, label: &str) -> i64 {
    let id = app
        .store
        .insert_many(vec![store::NewDecision {
            spec: Some("greeting".into()),
            question_hash: verdict::question_hash(&greeting()),
            question: "{}".into(),
            calibration_id: "previous-model".into(),
            state_hash: verdict::short_hash(state),
            state: state.into(),
            answer: vec![0.5, 0.5],
            verdict: "unsure".into(),
            endpoint: "d3".into(),
        }])
        .await
        .unwrap()[0];
    assert!(app.store.set_label(id, label).await.unwrap());

    id
}

fn indices(items: &[service::TriageItem]) -> Vec<usize> {
    items.iter().map(|item| item.index).collect()
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
    let labelled = label_under_old_model(&app, "maybe-1", "yes").await;
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

    for learning in ["explore = 0.0", "explore = 1.0\nreport = false"] {
        let app = fixture(learning).await;
        let decision = spec(&app, "no-1").await;
        assert_eq!(decision.verdict, "no", "{learning}");
        assert!(!decision.explored && !decision.report, "{learning}");
    }
}

#[tokio::test]
async fn learn_false_returns_the_model_alone() {
    let app = fixture("explore = 1.0").await;
    label_under_old_model(&app, "maybe-1", "yes").await;
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
    let calibration_id = verdict::calibration_id(
        &verdict::question_hash(&greeting()),
        domain::config::BackendKind::Systemone,
        &app.mock.uri(),
    );
    for i in 1..=24 {
        let id = spec(&app, &format!("yes-{i}")).await.id;
        app.service.label(id, "yes".into()).await.unwrap();
    }
    assert!(
        app.store
            .calibration(&calibration_id, Some("greeting"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(!spec(&app, "maybe-1").await.calibrated);

    let id = spec(&app, "yes-25").await.id;
    let before = requests(&app.mock).await.len();
    app.service.label(id, "yes".into()).await.unwrap();
    assert_eq!(
        requests(&app.mock).await.len(),
        before,
        "a refit uses cached answers only"
    );
    let (bands, report) = app
        .store
        .calibration(&calibration_id, Some("greeting"))
        .await
        .unwrap()
        .unwrap();
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
    let calibration_id = verdict::calibration_id(
        &verdict::question_hash(&greeting()),
        domain::config::BackendKind::Systemone,
        &app.mock.uri(),
    );
    // Labels set straight in the store skip the refit, as a failed probe or a busy refit would.
    for i in 1..=21 {
        let id = spec(&app, &format!("yes-{i}")).await.id;
        app.store.set_label(id, "yes").await.unwrap();
    }
    let id = spec(&app, "yes-22").await.id;
    app.service.label(id, "yes".into()).await.unwrap();
    let (_, report) = app
        .store
        .calibration(&calibration_id, Some("greeting"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.examples, 22);
}

#[tokio::test]
async fn calibrate_reasks_the_model_instead_of_remembering() {
    let app = fixture("").await;
    label_under_old_model(&app, "yes-old", "yes").await;
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
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(app.directory.path().join("db.sqlite")),
    )
    .await
    .unwrap();
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
