use super::*;

#[tokio::test]
async fn thread_goal_methods_share_state_with_goal_tools() {
    let workspace =
        std::env::temp_dir().join(format!("roder-goal-app-server-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(workspace).unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(app_server(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();
    let thread = start_thread(&client).await.thread;

    let set: ThreadGoalSetResult = request(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some("Ship shared goal state".to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: Some(Some(25)),
            })
            .unwrap(),
        ),
    )
    .await;
    let goal = set.goal.expect("created goal");
    assert_eq!(goal.objective, "Ship shared goal state");
    assert_eq!(goal.status, ThreadGoalStatus::Active);
    assert_eq!(goal.token_budget, Some(25));
    let updated =
        wait_for_notification(&mut notifications, "thread/goal/updated", Some(&thread.id)).await;
    assert_eq!(
        updated.params["goal"]["objective"],
        "Ship shared goal state"
    );

    let tool_get: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "get_goal".to_string(),
                arguments: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!tool_get.is_error);
    assert!(tool_get.text.contains("Ship shared goal state"));

    let tool_update: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "update_goal".to_string(),
                arguments: serde_json::json!({ "status": "blocked" }),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!tool_update.is_error);

    let get: ThreadGoalGetResult = request(
        &client,
        "thread/goal/get",
        Some(
            serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        get.goal.expect("goal after tool update").status,
        ThreadGoalStatus::Blocked
    );

    let invalid = request_error(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some(" ".to_string()),
                status: None,
                token_budget: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(invalid.message.contains("goal objective cannot be empty"));

    let clear: ThreadGoalClearResult = request(
        &client,
        "thread/goal/clear",
        Some(
            serde_json::to_value(ThreadGoalClearParams {
                thread_id: thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(clear.cleared);
    wait_for_notification(&mut notifications, "thread/goal/cleared", Some(&thread.id)).await;

    let get: ThreadGoalGetResult = request(
        &client,
        "thread/goal/get",
        Some(
            serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread.id,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(get.goal.is_none());
}

#[tokio::test]
async fn thread_goal_set_active_starts_idle_goal_turn() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(test_cwd()).unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(app_server(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let thread = start_thread(&client).await.thread;

    let set: ThreadGoalSetResult = request(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some("Start the idle goal immediately".to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: None,
            })
            .unwrap(),
        ),
    )
    .await;

    assert_eq!(
        set.goal.expect("goal should be active").status,
        ThreadGoalStatus::Active
    );
    wait_for_event(&mut events, &thread.id, "turn.started").await;
}

#[tokio::test]
async fn thread_goal_set_objective_steers_active_goal_turn() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(test_cwd()).unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(app_server(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let thread = start_thread(&client).await.thread;

    let _: TurnStartResult = start_turn(&client, &thread.id, "start a long turn").await;
    wait_for_event(&mut events, &thread.id, "turn.started").await;

    let set: ThreadGoalSetResult = request(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some("Aim the active turn at the new goal".to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: None,
            })
            .unwrap(),
        ),
    )
    .await;

    assert_eq!(
        set.goal.expect("goal should be active").status,
        ThreadGoalStatus::Active
    );
    let steered = wait_for_event(&mut events, &thread.id, "turn.steered").await;
    match steered.event {
        roder_api::events::RoderEvent::TurnSteered(event) => assert!(
            event
                .message
                .contains("Aim the active turn at the new goal"),
            "unexpected steering message: {}",
            event.message
        ),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn tools_call_can_create_and_get_goal() {
    let registry = build_default_registry(isolated_default_registry_config()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(app_server(runtime));
    let client = LocalAppClient::new(server);

    let thread_start = start_thread(&client).await;
    let created: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship slash goal",
                }),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!created.is_error, "create_goal failed: {created:?}");
    assert!(created.text.contains("Ship slash goal"));

    let duplicate: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship replacement goal",
                    "token_budget": 200,
                }),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(
        duplicate.is_error,
        "duplicate create_goal succeeded: {duplicate:?}"
    );
    assert!(
        duplicate
            .text
            .contains("cannot create a new goal because this thread has an unfinished goal")
    );

    let current: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "get_goal".to_string(),
                arguments: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!current.is_error, "get_goal failed: {current:?}");
    assert!(current.text.contains("Ship slash goal"));

    let completed: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "update_goal".to_string(),
                arguments: serde_json::json!({ "status": "complete" }),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(
        !completed.is_error,
        "update_goal complete failed: {completed:?}"
    );
    assert_eq!(completed.data["hasActiveGoal"], false);

    // Completed goals must not block a new create on resume / next objective.
    let next: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id,
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship the release",
                    "token_budget": 24000,
                }),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(
        !next.is_error,
        "create_goal after complete failed: {next:?}"
    );
    assert!(next.text.contains("Ship the release"));
    assert_eq!(next.data["hasActiveGoal"], true);
    assert_eq!(next.data["goal"]["status"], "active");
}

#[tokio::test]
async fn paused_goal_tool_is_shared_and_rejects_replacement_over_native_transport() {
    let registry = build_default_registry(isolated_default_registry_config()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(app_server(runtime)));
    let thread = start_thread(&client).await.thread;
    let created: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "create_goal".into(),
                arguments: serde_json::json!({"objective":"Keep this unfinished goal"}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!created.is_error);
    let paused: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "update_goal".into(),
                arguments: serde_json::json!({"status":"paused"}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!paused.is_error);
    assert_eq!(paused.data["goal"]["status"], "paused");
    let get: ThreadGoalGetResult = request(
        &client,
        "thread/goal/get",
        Some(
            serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(get.goal.unwrap().status, ThreadGoalStatus::Paused);
    let replacement: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id,
                tool_name: "create_goal".into(),
                arguments: serde_json::json!({"objective":"Discard old work"}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(replacement.is_error);
    assert!(replacement.text.contains("unfinished goal"));
}
