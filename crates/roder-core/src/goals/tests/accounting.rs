use super::*;

#[tokio::test]
async fn goal_usage_marks_budget_limited() {
    let runtime = runtime();
    let thread_id = "thread-budget".to_string();
    runtime
        .goals
        .create_thread_goal(&thread_id, "Spend budget".to_string(), Some(10))
        .await
        .unwrap();
    let goal = runtime
        .goals
        .account_turn_usage(&thread_id, 11, Duration::seconds(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.tokens_used, 11);
    assert_eq!(goal.status, ThreadGoalStatus::BudgetLimited);
}

#[tokio::test]
async fn budget_precedes_pause_block_and_resume_but_allows_completion() {
    let runtime = runtime();
    let id = "budget-precedence".to_string();
    runtime
        .goals
        .create_thread_goal(&id, "Budget".into(), Some(10))
        .await
        .unwrap();
    runtime.goals.begin_turn(&id, "turn", false).await.unwrap();
    runtime
        .goals
        .record_turn_usage(&id, "turn", 12)
        .await
        .unwrap();
    for status in [
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::Active,
    ] {
        let goal = runtime
            .thread_goal_set(
                &id,
                ThreadGoalPatch {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(goal.status, ThreadGoalStatus::BudgetLimited);
        assert_eq!(goal.tokens_used, 12);
    }
    let completed = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Complete),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, ThreadGoalStatus::Complete);
}

#[tokio::test]
async fn reducing_budget_limits_immediately_and_increasing_requires_resume() {
    let runtime = runtime();
    let id = "reduce-budget".to_string();
    runtime
        .goals
        .create_thread_goal(&id, "Budget".into(), Some(100))
        .await
        .unwrap();
    runtime
        .goals
        .account_turn_usage(&id, 40, Duration::ZERO)
        .await
        .unwrap();
    let goal = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                token_budget: Some(Some(30)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::BudgetLimited);
    let goal = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                token_budget: Some(Some(60)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::BudgetLimited);
    let goal = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Active),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Active);
}

#[tokio::test]
async fn model_completion_flushes_usage_and_stops_accounting_wrapup() {
    let runtime = runtime();
    let id = "complete-usage".to_string();
    runtime.goals.begin_turn(&id, "turn", false).await.unwrap();
    runtime
        .goals
        .record_turn_usage(&id, "turn", 25)
        .await
        .unwrap();
    runtime
        .goals
        .create_thread_goal(&id, "Created mid-turn".into(), Some(100))
        .await
        .unwrap();
    runtime
        .goals
        .record_turn_usage(&id, "turn", 30)
        .await
        .unwrap();
    let completed = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Complete),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.tokens_used, 30);
    runtime
        .goals
        .record_turn_usage(&id, "turn", 50)
        .await
        .unwrap();
    runtime.goals.finish_turn(&id, "turn", None).await.unwrap();
    assert_eq!(
        runtime
            .thread_goal_get(&id)
            .await
            .unwrap()
            .unwrap()
            .tokens_used,
        30
    );
}

#[tokio::test]
async fn descendant_tokens_roll_up_once_through_nested_agents() {
    use crate::teams::{TeamState, lead_member, teammate_member};
    use roder_api::policy_mode::PolicyMode;
    use roder_api::teams::AgentTeamDisplayMode;
    let dir = tempfile::tempdir().unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Runtime::new(
        builder.build().unwrap(),
        crate::RuntimeConfig {
            team_data_dir: Some(dir.path().into()),
            ..Default::default()
        },
    )
    .unwrap();
    let root = "root".to_string();
    let child = "child".to_string();
    let grandchild = "grandchild".to_string();
    let mut child_member = teammate_member(
        "child".into(),
        "Child".into(),
        child.clone(),
        None,
        None,
        PolicyMode::Default,
    );
    child_member.parent_thread_id = Some(root.clone());
    let mut grandchild_member = teammate_member(
        "grandchild".into(),
        "Grandchild".into(),
        grandchild.clone(),
        None,
        None,
        PolicyMode::Default,
    );
    grandchild_member.parent_thread_id = Some(child.clone());
    let now = OffsetDateTime::now_utc();
    runtime
        .teams
        .insert(TeamState {
            id: "team".into(),
            lead_thread_id: root.clone(),
            display_mode: AgentTeamDisplayMode::Auto,
            members: vec![
                lead_member(root.clone(), None, None, PolicyMode::Default),
                child_member,
                grandchild_member,
            ],
            mailbox: vec![],
            tasks: vec![],
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    runtime
        .goals
        .create_thread_goal(&root, "Root work".into(), Some(50))
        .await
        .unwrap();
    runtime
        .goals
        .create_thread_goal(&child, "Child work".into(), Some(100))
        .await
        .unwrap();
    runtime
        .goals
        .begin_turn(&root, "root-turn", false)
        .await
        .unwrap();
    runtime
        .record_goal_token_usage(&grandchild, "child-turn", 60)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .thread_goal_get(&root)
            .await
            .unwrap()
            .unwrap()
            .tokens_used,
        60
    );
    assert_eq!(
        runtime
            .thread_goal_get(&root)
            .await
            .unwrap()
            .unwrap()
            .status,
        ThreadGoalStatus::BudgetLimited
    );
    assert_eq!(
        runtime
            .thread_goal_get(&child)
            .await
            .unwrap()
            .unwrap()
            .tokens_used,
        60
    );
}

#[tokio::test]
async fn persisted_goal_reloads_with_usage_and_status_and_clears() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(roder_ext_jsonl_thread_store::store::JsonlThreadStore {
        base_path: dir.path().into(),
    });
    let id = "persisted-goal".to_string();
    let controller = RuntimeGoalController::new(EventBus::new(64), Some(store.clone()));
    controller
        .create_thread_goal(&id, "Durable goal".into(), Some(100))
        .await
        .unwrap();
    controller
        .account_turn_usage(&id, 40, Duration::seconds(7))
        .await
        .unwrap();
    controller
        .set_thread_goal(
            &id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Paused),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let reloaded = RuntimeGoalController::new(EventBus::new(64), Some(store.clone()));
    let goal = reloaded.get_thread_goal(&id).await.unwrap().unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Paused);
    assert_eq!(goal.tokens_used, 40);
    assert_eq!(goal.time_used_seconds, 7);
    assert!(
        reloaded
            .create_thread_goal(&id, "Replacement".into(), None)
            .await
            .is_err()
    );
    reloaded.clear_thread_goal(&id).await.unwrap();
    let cleared = RuntimeGoalController::new(EventBus::new(64), Some(store));
    assert!(cleared.get_thread_goal(&id).await.unwrap().is_none());
}
