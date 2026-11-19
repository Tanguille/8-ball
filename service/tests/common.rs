#![allow(
    dead_code,
    reason = "each test crate uses its own subset of these helpers"
)]

use std::{sync::Arc, time::Duration};

use serde_json::{Map, Value, json};
use sqlx::Connection;
use wiremock::{MockServer, matchers};

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
