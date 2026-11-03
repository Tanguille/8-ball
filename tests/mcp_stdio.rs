use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

#[tokio::test]
async fn handler_registers_tools_instructions_and_fail_open_results() {
    for (status, probability) in [(200, 0.97), (500, 0.97), (200, 2.0)] {
        let mock = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/systemone"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(
                    json!({"answers":{"greeting":{"type":"noul","noul":probability}}}),
                ),
            )
            .mount(&mock)
            .await;
        let config: eightball::config::Config =
            toml::from_str(
                &format!(
                    "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
                    mock.uri(),
                ),
            )
            .unwrap();
        let endpoint =
            eightball::backend::Endpoint::new("d3", &config.endpoints["d3"], None).unwrap();
        let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
        let service = std::sync::Arc::new(eightball::service::Service::new(
            config,
            [("d3".into(), endpoint)].into(),
            eightball::store::Store::open(&directory.path().join("db.sqlite"))
                .await
                .unwrap(),
        ));
        let (server_io, client_io) = tokio::io::duplex(16_384);
        let task = tokio::spawn(async move {
            eightball::mcp::Mcp::new(service)
                .serve(server_io)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let client = ().serve(client_io).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|text| text.name.as_ref()).collect();
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
                    .filter_map(|content| content.as_text())
                    .any(|text| text.text.contains("Fail open"))
            );
        }
        client.cancel().await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn binary_stdio_lists_tools_and_preserves_fail_open_errors() {
    for status in [200, 500] {
        let mock = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/systemone"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"answers":{"greeting":{"type":"noul","noul":0.97}}})),
            )
            .mount(&mock)
            .await;
        let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
        let endpoint_config = directory.path().join("config.toml");
        let database_path = directory.path().join("db.sqlite");
        std::fs::write(
            &endpoint_config,
            format!("default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'", mock.uri()),
        )
        .unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
        command
            .arg("mcp")
            .arg("--config")
            .arg(endpoint_config)
            .arg("--db")
            .arg(database_path)
            .kill_on_drop(true);
        let client =
            ().serve(rmcp::transport::TokioChildProcess::new(command).unwrap())
                .await
                .unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|text| text.name.as_ref()).collect();
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
                    .filter_map(|content| content.as_text())
                    .any(|text| text.text.contains("Fail open"))
            );
        }
        client.cancel().await.unwrap();
    }
}
