use super::*;

fn make_controller(directory: &std::path::Path) -> RuntimeGoalController {
    RuntimeGoalController::new(
        EventBus::new(64),
        Some(Arc::new(
            roder_ext_jsonl_thread_store::store::JsonlThreadStore {
                base_path: directory.to_path_buf(),
            },
        )),
    )
}

async fn create_goal(controller: &RuntimeGoalController, id: &ThreadId) -> ThreadGoal {
    controller
        .create_thread_goal(id, "Durable accounting".to_string(), Some(100))
        .await
        .unwrap()
}

async fn write_durable_goal(controller: &RuntimeGoalController, goal: &ThreadGoal) {
    tokio::fs::write(
        controller.goal_path(&goal.thread_id).unwrap(),
        serde_json::to_vec_pretty(goal).unwrap(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn committed_usage_write_error_does_not_request_a_second_own_or_ancestor_charge() {
    for recipient in ["own", "ancestor"] {
        let directory = tempfile::tempdir().unwrap();
        let controller = make_controller(directory.path());
        let id = recipient.to_string();
        let initial = create_goal(&controller, &id).await;
        controller.begin_turn(&id, "turn", false).await.unwrap();
        let _guard = controller.mutation.lock().await;
        let mut attempts = 0;
        for _ in 0..2 {
            attempts += 1;
            let mut desired = controller.load_goal(&id).await.unwrap().unwrap();
            desired.tokens_used += 25;
            desired.time_used_seconds += 3;
            desired.updated_at += Duration::seconds(1);
            let result = controller
                .persist_usage_goal(&desired, async {
                    write_durable_goal(&controller, &desired).await;
                    Err(anyhow::anyhow!("injected post-commit error"))
                })
                .await;
            if result.is_ok() {
                break;
            }
        }
        assert_eq!(attempts, 1, "{recipient} must not request a retry");
        let cached = controller.load_goal(&id).await.unwrap().unwrap();
        assert_eq!(cached.tokens_used, 25);
        assert_eq!(cached.time_used_seconds, 3);
        assert_eq!(cached.created_at, initial.created_at);
        drop(_guard);
        match recipient {
            "own" => {
                controller.record_turn_usage(&id, "turn", 10).await.unwrap();
            }
            "ancestor" => {
                controller.record_descendant_usage(&id, 10).await.unwrap();
            }
            _ => unreachable!(),
        }
        let reloaded = make_controller(directory.path());
        assert_eq!(
            reloaded
                .get_thread_goal(&id)
                .await
                .unwrap()
                .unwrap()
                .tokens_used,
            35
        );
    }
}

#[tokio::test]
async fn precommit_failure_keeps_the_original_error_and_remains_retryable() {
    let directory = tempfile::tempdir().unwrap();
    let controller = make_controller(directory.path());
    let id = "precommit".to_string();
    let initial = create_goal(&controller, &id).await;
    let mut desired = initial.clone();
    desired.tokens_used = 25;
    let original = anyhow::Error::new(std::io::Error::other("injected precommit error"));
    let _guard = controller.mutation.lock().await;
    let error = controller
        .persist_usage_goal(&desired, async { Err(original) })
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(error.to_string(), "injected precommit error");
    assert_eq!(controller.load_goal(&id).await.unwrap(), Some(initial));
    controller
        .persist_usage_goal(&desired, controller.store_goal(desired.clone()))
        .await
        .unwrap();
    assert_eq!(controller.load_goal(&id).await.unwrap(), Some(desired));
}

#[tokio::test]
async fn postcommit_confirmation_rejects_a_different_goal_generation() {
    let directory = tempfile::tempdir().unwrap();
    let controller = make_controller(directory.path());
    let id = "different-generation".to_string();
    let mut desired = create_goal(&controller, &id).await;
    desired.tokens_used = 25;
    let mut replacement = desired.clone();
    replacement.created_at += Duration::seconds(1);
    let _guard = controller.mutation.lock().await;
    let error = controller
        .persist_usage_goal(&desired, async {
            write_durable_goal(&controller, &replacement).await;
            Err(anyhow::anyhow!("write failed"))
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "write failed");
}

#[tokio::test]
async fn postcommit_confirmation_requires_the_entire_goal_to_match() {
    let directory = tempfile::tempdir().unwrap();
    let controller = make_controller(directory.path());
    let id = "different-goal-state".to_string();
    let mut desired = create_goal(&controller, &id).await;
    desired.tokens_used = 25;
    let _guard = controller.mutation.lock().await;
    for field in [
        "tokens",
        "seconds",
        "status",
        "objective",
        "budget",
        "updated_at",
    ] {
        let mut persisted = desired.clone();
        match field {
            "tokens" => persisted.tokens_used += 1,
            "seconds" => persisted.time_used_seconds += 1,
            "status" => persisted.status = ThreadGoalStatus::Paused,
            "objective" => persisted.objective = "Another objective".to_string(),
            "budget" => persisted.token_budget = Some(200),
            "updated_at" => persisted.updated_at += Duration::seconds(1),
            _ => unreachable!(),
        }
        let error = controller
            .persist_usage_goal(&desired, async {
                write_durable_goal(&controller, &persisted).await;
                Err(anyhow::anyhow!("write failed"))
            })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "write failed", "{field}");
    }
}

#[tokio::test]
async fn unconfirmable_commit_preserves_the_original_error() {
    let directory = tempfile::tempdir().unwrap();
    let controller = make_controller(directory.path());
    let id = "unreadable-goal".to_string();
    let desired = create_goal(&controller, &id).await;
    let _guard = controller.mutation.lock().await;
    let error = controller
        .persist_usage_goal(&desired, async {
            tokio::fs::write(controller.goal_path(&id).unwrap(), b"{invalid")
                .await
                .unwrap();
            Err(anyhow::anyhow!("original write failure"))
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "original write failure");
}
