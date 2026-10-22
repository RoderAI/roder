use super::*;

fn tier_runtime(root: &std::path::Path, engine: Arc<ImageRecordingEngine>) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: root.to_path_buf(),
    }));
    Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap())
}

async fn captured_tier(
    client: &LocalAppClient,
    engine: &ImageRecordingEngine,
    thread: &str,
    tier: Option<&str>,
) -> Option<String> {
    engine.requests.lock().await.clear();
    let mut events = client.subscribe_events();
    let _: TurnStartResult = request(
        client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": thread, "input": [{"type": "text", "text": "hello"}],
            "serviceTier": tier,
        })),
    )
    .await;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.thread_id.as_deref() != Some(thread) {
            continue;
        }
        if event.kind == "turn.failed" {
            panic!("turn failed: {:?}", event.event);
        }
        if event.kind == "turn.completed" {
            break;
        }
    }
    // Completion is emitted before admission is released; wait until a new turn can start.
    for attempt in 0..100 {
        let read: ThreadReadResult = request(
            client,
            "thread/read",
            Some(serde_json::json!({"threadId": thread})),
        )
        .await;
        if read.thread.unwrap().status.kind == "idle" {
            break;
        }
        assert!(attempt < 99, "thread did not become idle");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    engine
        .requests
        .lock()
        .await
        .first()
        .unwrap()
        .runtime
        .service_tier
        .clone()
}

#[tokio::test]
async fn service_tier_thread_defaults_turn_overrides_and_restart() {
    let root = std::env::temp_dir().join(format!("roder-service-tiers-{}", uuid::Uuid::new_v4()));
    let engine = Arc::new(ImageRecordingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let runtime = tier_runtime(&root, engine.clone());
    let client = LocalAppClient::new(Arc::new(app_server(runtime.clone())));
    let workspace = create_workspace_for_path(&client, std::path::Path::new(&test_cwd())).await;
    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "workspaceId": workspace.workspace_id, "rootId": workspace.root_id,
            "serviceTier": "ultrafast",
        })),
    )
    .await;
    let id = started.thread.id;
    assert_eq!(started.thread.service_tier.as_deref(), Some("ultrafast"));
    let other = start_thread(&client).await.thread.id;
    assert_eq!(
        captured_tier(&client, &engine, &id, None).await.as_deref(),
        Some("ultrafast")
    );
    assert_eq!(
        captured_tier(&client, &engine, &id, Some("default"))
            .await
            .as_deref(),
        Some("default")
    );
    assert_eq!(
        captured_tier(&client, &engine, &id, None).await.as_deref(),
        Some("ultrafast")
    );
    assert_eq!(captured_tier(&client, &engine, &other, None).await, None);

    let changed: serde_json::Value = request(
        &client,
        "thread/set_service_tier",
        Some(serde_json::json!({
            "threadId": id, "serviceTier": "flex",
        })),
    )
    .await;
    assert_eq!(changed["serviceTier"], "flex");
    drop(client);
    drop(runtime);

    // A fresh runtime and server read the persisted default from the JSONL store.
    let runtime = tier_runtime(&root, engine.clone());
    let client = LocalAppClient::new(Arc::new(app_server(runtime)));
    let read: ThreadReadResult = request(
        &client,
        "thread/read",
        Some(serde_json::json!({"threadId": id})),
    )
    .await;
    assert_eq!(read.thread.unwrap().service_tier.as_deref(), Some("flex"));
    assert_eq!(
        captured_tier(&client, &engine, &id, None).await.as_deref(),
        Some("flex")
    );
    let cleared: serde_json::Value = request(
        &client,
        "thread/set_service_tier",
        Some(serde_json::json!({
            "threadId": id, "serviceTier": null,
        })),
    )
    .await;
    assert!(cleared["serviceTier"].is_null());
    assert_eq!(captured_tier(&client, &engine, &id, None).await, None);
    let read: serde_json::Value = request(
        &client,
        "thread/read",
        Some(serde_json::json!({"threadId": id})),
    )
    .await;
    assert!(read["thread"].get("serviceTier").is_none());
    let invalid = request_error(
        &client,
        "thread/set_service_tier",
        Some(serde_json::json!({
            "threadId": id, "serviceTier": "  ",
        })),
    )
    .await;
    assert_eq!(invalid.code, -32602);
    let missing = request_error(
        &client,
        "thread/set_service_tier",
        Some(serde_json::json!({
            "threadId": "missing", "serviceTier": "ultrafast",
        })),
    )
    .await;
    assert_eq!(missing.code, -32602);
    drop(client);
    let _ = std::fs::remove_dir_all(root);
}
