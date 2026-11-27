#![allow(
    dead_code,
    reason = "each test crate uses its own subset of these helpers"
)]

use std::{sync::Arc, time::Duration};

use serde_json::{Map, Value, json};
use sqlx::Connection;
use wiremock::{MockServer, matchers};

use domain::verdict;
use service::Service;
use store::Store;

/// A fresh store in a temporary directory, which the store lives in.
pub async fn store() -> (Store, tempfile::TempDir) {
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();

    (store, directory)
}

pub async fn service(config: domain::config::Config) -> (Arc<Service>, Store, tempfile::TempDir) {
    let (store, directory) = store().await;

    (
        Arc::new(Service::new(config, store.clone())),
        store,
        directory,
    )
}

/// Config TOML with the `systemone` endpoint `d3` at `url` as the default. `extra` goes after the endpoint table,
/// so it can add endpoint keys before its own tables.
#[must_use]
pub fn config(url: &str, extra: &str) -> String {
    format!("default='d3'\n[endpoints.d3]\nurl='{url}'\nkind='systemone'\n{extra}")
}

/// A service with `systemone` as the default endpoint `d3`; `extra` as in `config`.
pub async fn d3(extra: &str) -> (Arc<Service>, Store, MockServer, tempfile::TempDir) {
    let mock = systemone().await;
    let config = toml::from_str(&config(&mock.uri(), extra)).unwrap();
    let (service, store, directory) = service(config).await;

    (service, store, mock, directory)
}

/// The first result of an ask `request`, given as JSON.
pub async fn ask(service: &Service, request: Value) -> service::Decision {
    let request = serde_json::from_value(request).unwrap();
    service.ask(request).await.unwrap().results.remove(0)
}

/// The response to a triage `request`, given as JSON.
pub async fn triage(service: &Service, request: Value) -> service::TriageResponse {
    let request = serde_json::from_value(request).unwrap();
    service.triage(request).await.unwrap()
}

/// Serves the REST and MCP router on a free local port and returns its base URL.
pub async fn serve(service: Arc<Service>, api_key: Option<&str>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = service::http::router(
        service,
        api_key,
        &tokio_util::sync::CancellationToken::new(),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    url
}

/// A systemone mock that answers noul by state (`no-*` 0.03, `maybe-*` 0.5, else 0.97, `slow-*` after 5 s) and
/// choices with their first option. State `fail` gets a 500.
pub async fn systemone() -> MockServer {
    let mock = MockServer::start().await;
    wiremock::Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/systemone"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let state = body["state"].as_str().unwrap();
            if state == "fail" {
                return wiremock::ResponseTemplate::new(500);
            }
            let noul = if state.starts_with("no-") {
                0.03
            } else if state.starts_with("maybe-") {
                0.5
            } else {
                0.97
            };
            let answers: Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = if question["type"] == "choice" {
                        let probabilities: Map<String, Value> = question["criteria"]
                            .as_object()
                            .unwrap()
                            .keys()
                            .enumerate()
                            .map(|(i, name)| {
                                (name.clone(), json!(if i == 0 { 0.99 } else { 0.01 }))
                            })
                            .collect();
                        json!({"type":"choice","probabilities":probabilities})
                    } else {
                        json!({"type":"noul","noul":noul})
                    };

                    (id.clone(), answer)
                })
                .collect();
            let response =
                wiremock::ResponseTemplate::new(200).set_body_json(json!({"answers":answers}));
            if state.starts_with("slow-") {
                response.set_delay(Duration::from_secs(5))
            } else {
                response
            }
        })
        .mount(&mock)
        .await;

    mock
}

/// Model calls only: the first request also fetches endpoint info.
pub async fn requests(mock: &MockServer) -> Vec<Value> {
    mock.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

#[must_use]
pub fn greeting() -> domain::question::Question {
    serde_json::from_value(json!({"id":"greeting","type":"noul","instructions":"Greeting?"}))
        .unwrap()
}

/// The stored fit of the `greeting` spec on an endpoint of `kind` with `fingerprint`.
pub async fn greeting_fit(
    store: &Store,
    kind: domain::config::BackendKind,
    fingerprint: &str,
) -> Option<(verdict::Bands, verdict::Report)> {
    let calibration_id =
        verdict::calibration_id(&verdict::question_hash(&greeting()), kind, fingerprint);

    store
        .calibration(&calibration_id, Some("greeting"))
        .await
        .unwrap()
}

/// A labelled `greeting` decision under a model the service never uses, so only `remember` can find it.
pub async fn label_under_old_model(store: &Store, state: &str, label: &str) -> i64 {
    let id = store
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
    assert!(store.set_label(id, label).await.unwrap());

    id
}

#[must_use]
pub fn indices(items: &[service::TriageItem]) -> Vec<usize> {
    items.iter().map(|item| item.index).collect()
}

/// A raw connection to the database of `common::store`, for what the store API does not expose.
pub async fn connection(directory: &tempfile::TempDir) -> sqlx::SqliteConnection {
    sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(directory.path().join("db.sqlite")),
    )
    .await
    .unwrap()
}
