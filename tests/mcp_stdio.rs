use eightball::{backend::Endpoint, config::Config, mcp::Mcp, service::Service, store::Store};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn handler_registers_tools_instructions_and_fail_open_results() {
    for (status, probability) in [(200, 0.97), (500, 0.97), (200, 2.0)] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(
                    json!({"answers":{"greeting":{"type":"noul","noul":probability}}}),
                ),
            )
            .mount(&mock)
            .await;
        let config:Config=toml::from_str(&format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",mock.uri())).unwrap();
        let ep = Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
        let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
        let service = Arc::new(Service::new(
            config,
            [("d3".into(), ep)].into(),
            Store::open(&dir.path().join("db.sqlite")).await.unwrap(),
        ));
        let (server_io, client_io) = tokio::io::duplex(16_384);
        let task = tokio::spawn(async move {
            Mcp::new(service)
                .serve(server_io)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let client = ().serve(client_io).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(names, ["decision_ask", "decision_triage"]);
        assert!(
            client
                .peer_info()
                .unwrap()
                .instructions
                .as_deref()
                .unwrap()
                .contains("greeting")
        );
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
        if status == 200 && probability < 1.0 {
            assert_eq!(
                result.structured_content.as_ref().unwrap()["results"][0]["verdict"],
                "yes"
            );
        } else {
            assert_eq!(result.is_error, Some(true));
            assert!(
                result
                    .content
                    .iter()
                    .filter_map(|c| c.as_text())
                    .any(|t| t.text.contains("Fail open"))
            );
        }
        client.cancel().await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn binary_stdio_lists_tools_and_preserves_fail_open_errors() {
    use rmcp::transport::TokioChildProcess;
    for status in [200, 500] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"answers":{"greeting":{"type":"noul","noul":0.97}}})),
            )
            .mount(&mock)
            .await;
        let dir = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        let db = dir.path().join("db.sqlite");
        std::fs::write(&cfg,format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",mock.uri())).unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
        command
            .arg("mcp")
            .arg("--config")
            .arg(cfg)
            .arg("--db")
            .arg(db)
            .kill_on_drop(true);
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(names, ["decision_ask", "decision_triage"]);
        assert!(
            client
                .peer_info()
                .unwrap()
                .instructions
                .as_deref()
                .unwrap()
                .contains("greeting")
        );
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
        if status == 200 {
            assert_eq!(
                result.structured_content.unwrap()["results"][0]["verdict"],
                "yes"
            );
        } else {
            assert_eq!(result.is_error, Some(true));
            assert!(
                result
                    .content
                    .iter()
                    .filter_map(|c| c.as_text())
                    .any(|t| t.text.contains("Fail open"))
            );
        }
        client.cancel().await.unwrap();
    }
}
