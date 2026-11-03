use eightball::{backend::Endpoint, config::Config, http::router, service::Service, store::Store};
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn serve(
    mock: &MockServer,
    db: &Path,
) -> (String, CancellationToken, tokio::task::JoinHandle<()>) {
    let config:Config=toml::from_str(&format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",mock.uri())).unwrap();
    let ep = Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
    let service = Arc::new(Service::new(
        config,
        [("d3".into(), ep)].into(),
        Store::open(db).unwrap(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let token = CancellationToken::new();
    let app = router(service, Some("k"), &token);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, token, task)
}
#[tokio::test]
async fn authenticated_http_client_survives_server_restart() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"answers":{"greeting":{"type":"noul","noul":0.97}}})),
        )
        .mount(&mock)
        .await;
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    for restart in [false, true] {
        let (url, token, task) = serve(&mock, &db).await;
        let client = ()
            .serve(StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(format!("{url}/mcp"))
                    .auth_header("k"),
            ))
            .await
            .unwrap();
        assert_eq!(client.list_all_tools().await.unwrap().len(), 2);
        let result = client
            .call_tool(
                CallToolRequestParams::new("decision_ask").with_arguments(
                    json!({"state":"hi","spec":"greeting"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        let body = result.structured_content.unwrap();
        assert_eq!(body["results"][0]["verdict"], "yes");
        assert_eq!(body["results"][0]["cached"], restart);
        client.cancel().await.unwrap();
        token.cancel();
        task.abort();
    }
}
#[tokio::test]
async fn bearer_allows_non_loopback_host_header() {
    let mock = MockServer::start().await;
    let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let (url, token, task) = serve(&mock, &dir.path().join("db.sqlite")).await;
    let response=reqwest::Client::new().post(format!("{url}/mcp")).header("Host","8ball.example").header("Accept","application/json, text/event-stream").bearer_auth("k").json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap();
    assert_ne!(response.status(), 403);
    assert!(response.status().is_success(), "{}", response.status());
    token.cancel();
    task.abort();
}
