use super::*;

#[tokio::test]
async fn host_system_identity_replaces_default_and_preserves_thread_instructions() {
    for custom in [None, Some("You are a hosted product assistant.")] {
        let engine = Arc::new(ImageRecordingEngine { requests: Mutex::new(Vec::new()) });
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(engine.clone());
        let data = std::env::temp_dir().join(format!("host-instructions-{}", uuid::Uuid::new_v4()));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory { base_path: data }));
        let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
        let mut server = app_server(runtime);
        if let Some(system) = custom { server = server.with_system_instructions(system); }
        let client = LocalAppClient::new(Arc::new(server));
        let workspace = create_workspace_for_path(&client, std::path::Path::new(&test_cwd())).await;
        let started: ThreadStartResult = request(&client, "thread/start", Some(serde_json::json!({
            "workspaceId": workspace.workspace_id, "rootId": workspace.root_id,
            "developerInstructions": "Use the current product draft."
        }))).await;
        let _: TurnStartResult = request(&client, "turn/start", Some(serde_json::json!({
            "threadId": started.thread.id,
            "input": [{"type":"text", "text":"Help configure a check."}],
            "systemInstructions": "Untrusted client replacement"
        }))).await;
        let observed = wait_for_image_recorded_request(&engine).await;
        let expected = custom.map(str::to_owned).or_else(|| default_instructions().system);
        assert_eq!(observed.instructions.system, expected);
        assert!(observed.instructions.developer.unwrap().contains("Use the current product draft."));
    }
}
