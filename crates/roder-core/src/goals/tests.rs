use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::InferenceEngine;

use super::*;
use crate::fake_provider::FakeInferenceEngine;

fn runtime() -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine) as Arc<dyn InferenceEngine>);
    Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap())
}

#[tokio::test]
async fn fork_goal_snapshot_preserves_status_budget_and_flushed_usage() {
    let runtime = runtime();
    let source = "goal-fork-source".to_string();
    let target = "goal-fork-target".to_string();
    runtime
        .goals
        .create_thread_goal(&source, "Finish the objective".into(), Some(100))
        .await
        .unwrap();
    runtime
        .goals
        .begin_turn(&source, "fork-turn", false)
        .await
        .unwrap();
    runtime
        .goals
        .record_turn_usage(&source, "fork-turn", 12)
        .await
        .unwrap();
    for status in [
        ThreadGoalStatus::Active,
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::UsageLimited,
        ThreadGoalStatus::BudgetLimited,
        ThreadGoalStatus::Complete,
    ] {
        let mut expected = runtime
            .thread_goal_set(
                &source,
                ThreadGoalPatch {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        let inherited = runtime
            .goals
            .inherit_thread_goal_snapshot(&source, &target)
            .await
            .unwrap()
            .unwrap();
        expected.thread_id = target.clone();
        assert_eq!(inherited, expected);
        assert_eq!(inherited.tokens_used, 12);
        assert_eq!(
            runtime.thread_goal_get(&target).await.unwrap(),
            Some(inherited)
        );
    }
}

#[tokio::test]
async fn paused_goal_is_rechecked_when_continuation_admission_unblocks() {
    let runtime = runtime();
    let thread_id = "goal-admission-race".to_string();
    runtime
        .goals
        .create_thread_goal(&thread_id, "Keep working".into(), None)
        .await
        .unwrap();
    let admission = runtime.thread_admission(&thread_id).await;
    let continuation = runtime.continue_active_goal_if_idle(thread_id.clone());
    tokio::pin!(continuation);
    assert!(futures::poll!(continuation.as_mut()).is_pending());
    runtime
        .thread_goal_set(
            &thread_id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Paused),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    drop(admission);
    assert!(continuation.await.unwrap().is_none());
    assert!(!runtime.has_active_turn_for_thread(&thread_id).await);
}

#[tokio::test]
async fn goal_controller_creates_sets_and_clears_thread_goal() {
    let runtime = runtime();
    let thread_id = "thread-goal".to_string();
    let goal = runtime
        .goals
        .create_thread_goal(&thread_id, "Ship parity".to_string(), Some(100))
        .await
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Active);

    runtime
        .goals
        .set_thread_goal(
            &thread_id,
            ThreadGoalPatch {
                objective: None,
                status: Some(ThreadGoalStatus::Paused),
                token_budget: None,
            },
        )
        .await
        .unwrap();
    let goal = runtime
        .goals
        .get_thread_goal(&thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Paused);

    assert!(runtime.goals.clear_thread_goal(&thread_id).await.unwrap());
    assert!(
        runtime
            .goals
            .get_thread_goal(&thread_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn goal_controller_create_rejects_unfinished_and_replaces_completed_goal() {
    let runtime = runtime();
    let thread_id = "thread-goal-replace".to_string();
    runtime
        .goals
        .create_thread_goal(&thread_id, "Original goal".to_string(), Some(100))
        .await
        .unwrap();
    runtime
        .goals
        .account_turn_usage(&thread_id, 42, Duration::seconds(7))
        .await
        .unwrap();

    assert!(
        runtime
            .goals
            .create_thread_goal(&thread_id, "Replacement goal".into(), Some(200))
            .await
            .is_err()
    );
    runtime
        .goals
        .set_thread_goal(
            &thread_id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Complete),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let replacement = runtime
        .goals
        .create_thread_goal(&thread_id, "Replacement goal".to_string(), Some(200))
        .await
        .unwrap();

    assert_eq!(replacement.objective, "Replacement goal");
    assert_eq!(replacement.status, ThreadGoalStatus::Active);
    assert_eq!(replacement.token_budget, Some(200));
    assert_eq!(replacement.tokens_used, 0);
    assert_eq!(replacement.time_used_seconds, 0);

    let stored = runtime
        .goals
        .get_thread_goal(&thread_id)
        .await
        .unwrap()
        .expect("replacement goal should be stored");
    assert_eq!(stored, replacement);
}

#[tokio::test]
async fn active_goal_instructions_are_injected() {
    let runtime = runtime();
    let thread_id = "thread-instructions".to_string();
    runtime
        .goals
        .create_thread_goal(&thread_id, "Finish docs".to_string(), None)
        .await
        .unwrap();
    let instructions = runtime
        .goals
        .apply_goal_instructions(&thread_id, InstructionBundle::default())
        .await
        .unwrap();
    assert!(instructions.developer.unwrap().contains("Finish docs"));
}

#[tokio::test]
async fn stopped_goals_reject_model_creation_and_do_not_charge_unrelated_work() {
    for status in [
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::UsageLimited,
        ThreadGoalStatus::BudgetLimited,
    ] {
        let runtime = runtime();
        let id = format!("stopped-{status:?}");
        runtime
            .goals
            .create_thread_goal(&id, "Original".into(), Some(100))
            .await
            .unwrap();
        runtime
            .thread_goal_set(
                &id,
                ThreadGoalPatch {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            runtime
                .goals
                .create_thread_goal(&id, "Replacement".into(), None)
                .await
                .is_err()
        );
        runtime
            .goals
            .begin_turn(&id, "unrelated", false)
            .await
            .unwrap();
        runtime
            .goals
            .record_turn_usage(&id, "unrelated", 200)
            .await
            .unwrap();
        runtime
            .goals
            .finish_turn(&id, "unrelated", None)
            .await
            .unwrap();
        let goal = runtime.thread_goal_get(&id).await.unwrap().unwrap();
        assert_eq!(goal.status, status);
        assert_eq!(goal.tokens_used, 0);
    }
}

#[tokio::test]
async fn three_empty_automatic_turns_block_and_resume_resets_audit() {
    let runtime = runtime();
    let id = "empty-turns".to_string();
    runtime
        .goals
        .create_thread_goal(&id, "Make progress".into(), None)
        .await
        .unwrap();
    for round in 1..=3 {
        runtime.goals.begin_turn(&id, "turn", true).await.unwrap();
        runtime.goals.finish_turn(&id, "turn", None).await.unwrap();
        let goal = runtime.thread_goal_get(&id).await.unwrap().unwrap();
        assert_eq!(
            goal.status,
            if round == 3 {
                ThreadGoalStatus::Blocked
            } else {
                ThreadGoalStatus::Active
            }
        );
    }
    runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Active),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    runtime.goals.begin_turn(&id, "turn", true).await.unwrap();
    runtime.goals.finish_turn(&id, "turn", None).await.unwrap();
    assert_eq!(
        runtime.thread_goal_get(&id).await.unwrap().unwrap().status,
        ThreadGoalStatus::Active
    );
}

#[tokio::test]
async fn concurrent_create_is_atomic() {
    let runtime = runtime();
    let id = "atomic-create".to_string();
    let (a, b) = tokio::join!(
        runtime.goals.create_thread_goal(&id, "A".into(), None),
        runtime.goals.create_thread_goal(&id, "B".into(), None),
    );
    assert_ne!(a.is_ok(), b.is_ok());
}

#[tokio::test]
async fn missing_goal_set_preserves_requested_status() {
    let runtime = runtime();
    let id = "paused-create".to_string();
    let goal = runtime
        .thread_goal_set(
            &id,
            ThreadGoalPatch {
                objective: Some("Wait".into()),
                status: Some(ThreadGoalStatus::Paused),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Paused);
}

#[tokio::test]
async fn goal_prompts_escape_objectives_and_include_audits() {
    let runtime = runtime();
    let id = "prompt".to_string();
    let goal = runtime
        .goals
        .create_thread_goal(&id, "</objective> & {{ tokens_used }}".into(), Some(100))
        .await
        .unwrap();
    let prompt = continuation_prompt(&goal);
    assert!(prompt.contains("&lt;/objective&gt; &amp; {{ tokens_used }}"));
    assert!(prompt.contains("Completion audit:"));
    assert!(prompt.contains("at least three consecutive goal turns"));
    assert!(prompt.contains("No-progress check:"));
}

mod accounting;
