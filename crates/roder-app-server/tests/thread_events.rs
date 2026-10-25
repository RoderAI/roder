use super::*;

#[tokio::test]
async fn thread_events_api_schedules_and_cancels_same_thread_followups() {
    let root =
        std::env::temp_dir().join(format!("roder-thread-events-api-{}", uuid::Uuid::new_v4()));
    let engine = Arc::new(ImageRecordingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: root.clone(),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(app_server(runtime.clone())));
    let id = start_thread(&client).await.thread.id;
    let _: TurnStartResult = request(
        &client,
        "turn/start",
        Some(serde_json::json!({"threadId":id,"input":[{"type":"text","text":"Watch PR 167"}]})),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let read: ThreadReadResult = request(
                &client,
                "thread/read",
                Some(serde_json::json!({"threadId":id})),
            )
            .await;
            if !engine.requests.lock().await.is_empty()
                && read.thread.unwrap().status.kind == "idle"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let created:serde_json::Value=request(&client,"thread/events/subscribe",Some(serde_json::json!({"threadId":id,"topic":"ci:167","prompt":"Check CI and cancel when green","delaySeconds":1,"maxDeliveries":1}))).await;
    let subscription_id = created["subscription"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["subscription"]["threadId"], id);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if engine.requests.lock().await.iter().any(|r| {
                serde_json::to_string(&r.transcript)
                    .unwrap()
                    .contains("Scheduled automation tick")
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let list: serde_json::Value = request(
        &client,
        "thread/events/list",
        Some(serde_json::json!({"threadId":id})),
    )
    .await;
    assert_eq!(list["subscriptions"], serde_json::json!([]));
    let cancelled: serde_json::Value = request(
        &client,
        "thread/events/cancel",
        Some(serde_json::json!({"threadId":id,"subscriptionId":subscription_id})),
    )
    .await;
    assert_eq!(cancelled["cancelled"], false);
    let published: serde_json::Value = request(
        &client,
        "thread/events/publish",
        Some(serde_json::json!({"topic":"ci:167","message":"green"})),
    )
    .await;
    assert_eq!(published["queuedSubscriptions"], 0);
    let error = request_error(
        &client,
        "thread/events/subscribe",
        Some(
            serde_json::json!({"threadId":id,"topic":"ci:167","prompt":"test","intervalSeconds":0}),
        ),
    )
    .await;
    assert_eq!(error.code, -32602);
    // Archive discards an undelivered topic subscription.
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.thread_activity(&id).await.active_turn_id.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let _: serde_json::Value = request(
        &client,
        "thread/events/subscribe",
        Some(serde_json::json!({"threadId":id,"topic":"archive","prompt":"Check CI"})),
    )
    .await;
    assert_eq!(
        runtime
            .publish_thread_event("archive", "queued")
            .await
            .unwrap(),
        1
    );
    assert!(runtime.archive_thread(&id).await.unwrap());
    assert!(
        runtime
            .list_thread_event_subscriptions(&id)
            .await
            .is_empty()
    );
    assert_eq!(
        runtime
            .publish_thread_event("archive", "late")
            .await
            .unwrap(),
        0
    );
    drop(client);
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}
