use std::sync::Arc;

use service::Service;
use store::Store;

pub async fn service(config: domain::config::Config) -> (Arc<Service>, Store, tempfile::TempDir) {
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();

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
