use rmcp::ServiceExt;
use serde_json::json;
use tempfile::Builder;
use wiremock::MockServer;

#[path = "support/common.rs"]
mod common;

async fn serve(mock: &MockServer, database_path: &std::path::Path) -> String {
    let config: domain::config::Config =
        toml::from_str(
            &format!(
                "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
                mock.uri(),
            ),
        )
        .unwrap();
    let service = service::Service::new(config, store::Store::open(database_path).await.unwrap());

    common::serve(std::sync::Arc::new(service), Some("k")).await
}
#[tokio::test]
async fn mcp_over_http_with_key_reads_cache_after_restart() {
    let mock = common::systemone().await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let database_path = directory.path().join("db.sqlite");
    for restart in [false, true] {
        let url = serve(&mock, &database_path).await;
        let client = ()
            .serve(rmcp::transport::StreamableHttpClientTransport::from_config(
                rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(format!("{url}/mcp"))
                    .auth_header("k"),
            ))
            .await
            .unwrap();
        assert_eq!(client.list_all_tools().await.unwrap().len(), 5);
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("decision_ask")
                    .with_arguments(rmcp::object!({"state":"hi","spec":"greeting"})),
            )
            .await
            .unwrap();
        let body = result.structured_content.unwrap();
        assert_eq!(body["results"][0]["verdict"], "yes");
        assert_eq!(body["results"][0]["cached"], restart);
        client.cancel().await.unwrap();
    }
}
#[tokio::test]
async fn bearer_allows_non_loopback_host_header() {
    let mock = MockServer::start().await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let url = serve(&mock, &directory.path().join("db.sqlite")).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/mcp"))
        .header("Host", "8ball.example")
        .header("Accept", "application/json, text/event-stream")
        .bearer_auth("k")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
}
#[tokio::test]
async fn bearer_scheme_is_case_insensitive() {
    let mock = MockServer::start().await;
    let directory = Builder::new().prefix("8ball-").tempdir().unwrap();
    let url = serve(&mock, &directory.path().join("db.sqlite")).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/questions"))
        .header("Authorization", "bearer k")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let other_scheme = reqwest::Client::new()
        .post(format!("{url}/v1/questions"))
        .header("Authorization", "Basic k")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(other_scheme.status(), 401);
}
