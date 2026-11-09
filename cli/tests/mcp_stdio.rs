use rmcp::{RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService};
use wiremock::matchers;

async fn check_client(client: RunningService<RoleClient, ()>, healthy: bool) {
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|text| text.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "decision_ask",
            "decision_label",
            "decision_questions",
            "decision_review",
            "decision_triage"
        ]
    );
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
            CallToolRequestParams::new("decision_ask")
                .with_arguments(rmcp::object!({"state":"hi","spec":"greeting"})),
        )
        .await
        .unwrap();
    if healthy {
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

#[tokio::test]
async fn binary_stdio_lists_tools_and_preserves_fail_open_errors() {
    for (status, probability) in [(200, 0.97), (500, 0.97), (200, 2.0)] {
        let mock = wiremock::MockServer::start().await;
        wiremock::Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/systemone"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(
                serde_json::json!({"answers":{"greeting":{"type":"noul","noul":probability}}}),
            ))
            .mount(&mock)
            .await;
        let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
        let database_path = directory.path().join("db.sqlite");
        let text = format!(
            "default='d3'\n[endpoints.d3]\nurl='{}'\nkind='systemone'\n[specs.greeting]\ntype='noul'\ninstructions='Greeting?'",
            mock.uri()
        );
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
        // Config variables from the developer's shell would conflict with the source chosen below.
        command
            .arg("mcp")
            .arg("--db")
            .arg(database_path)
            .env_remove("EIGHTBALL_CONFIG")
            .env_remove("EIGHTBALL_CONFIG_TOML")
            .kill_on_drop(true);
        // Cover both config sources: a file when the model answers, the environment when it fails.
        if status == 200 {
            let endpoint_config = directory.path().join("config.toml");
            std::fs::write(&endpoint_config, text).unwrap();
            command.arg("--config").arg(endpoint_config);
        } else {
            command.env("EIGHTBALL_CONFIG_TOML", text);
        }
        let client =
            ().serve(rmcp::transport::TokioChildProcess::new(command).unwrap())
                .await
                .unwrap();
        check_client(client, status == 200 && probability < 1.0).await;
    }
}
