use super::*;

#[tokio::test]
async fn native_computer_name_collision_is_rejected_through_thread_start() {
    let runtime = external_tool_runtime(RuntimeConfig::default());
    let client = LocalAppClient::new(Arc::new(app_server(runtime)));
    let cwd = test_cwd();
    let workspace = create_workspace_for_path(&client, std::path::Path::new(&cwd)).await;
    let error = request_error(
        &client,
        "thread/start",
        Some(
            serde_json::to_value(ThreadStartParams {
                selection: None,
                model: None,
                model_provider: None,
                reasoning: None,
                workspace_id: workspace.workspace_id,
                root_id: Some(workspace.root_id),
                cwd: Some(cwd),
                tool_allowlist: None,
                developer_instructions: None,
                external_tools: Some(vec![roder_api::computer::computer_tool_spec()]),
                mcp_auth_token: None,
                runner: None,
                ephemeral: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(error.message.contains("computer is reserved"), "{error:?}");
}
