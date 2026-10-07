use std::sync::Arc;

use roder_api::events::{ThreadId, TurnId};
use roder_api::goals::ThreadGoalController;
use roder_api::policy_mode::PolicyMode;
use time::OffsetDateTime;
use tokio::sync::Mutex;

use super::*;

#[derive(Default)]
struct FakeGoalController {
    goal: Mutex<Option<ThreadGoal>>,
}

#[async_trait::async_trait]
impl ThreadGoalController for FakeGoalController {
    async fn get_thread_goal(&self, _thread_id: &ThreadId) -> anyhow::Result<Option<ThreadGoal>> {
        Ok(self.goal.lock().await.clone())
    }

    async fn create_thread_goal(
        &self,
        thread_id: &ThreadId,
        objective: String,
        token_budget: Option<i64>,
    ) -> anyhow::Result<ThreadGoal> {
        let now = OffsetDateTime::now_utc();
        let goal = ThreadGoal {
            thread_id: thread_id.clone(),
            objective,
            status: ThreadGoalStatus::Active,
            token_budget,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: now,
            updated_at: now,
        };
        *self.goal.lock().await = Some(goal.clone());
        Ok(goal)
    }

    async fn set_thread_goal(
        &self,
        _thread_id: &ThreadId,
        patch: ThreadGoalPatch,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let mut guard = self.goal.lock().await;
        let Some(goal) = guard.as_mut() else {
            return Ok(None);
        };
        if let Some(status) = patch.status {
            goal.status = status;
        }
        if let Some(objective) = patch.objective {
            goal.objective = objective;
        }
        if let Some(token_budget) = patch.token_budget {
            goal.token_budget = token_budget;
        }
        Ok(Some(goal.clone()))
    }

    async fn clear_thread_goal(&self, _thread_id: &ThreadId) -> anyhow::Result<bool> {
        Ok(self.goal.lock().await.take().is_some())
    }
}

#[tokio::test]
async fn goal_tools_create_get_and_complete_goal() {
    let controller = Arc::new(FakeGoalController::default());
    let create = CreateGoalTool;
    let get = GetGoalTool;
    let update = UpdateGoalTool;

    let created = create
        .execute(
            context(controller.clone()),
            call("create_goal", json!({ "objective": "Ship parity" })),
        )
        .await
        .unwrap();
    assert!(!created.is_error);
    assert_eq!(created.data["hasActiveGoal"], true);

    let current = get
        .execute(context(controller.clone()), call("get_goal", json!({})))
        .await
        .unwrap();
    assert!(current.text.contains("Ship parity"));

    let completed = update
        .execute(
            context(controller),
            call("update_goal", json!({ "status": "complete" })),
        )
        .await
        .unwrap();
    assert!(!completed.is_error);
    assert_eq!(completed.data["hasActiveGoal"], false);
    assert_eq!(completed.data["goal"]["status"], "complete");
}

#[tokio::test]
async fn create_goal_fails_when_active_goal_exists() {
    let controller = Arc::new(FakeGoalController::default());
    let create = CreateGoalTool;

    let original = create
        .execute(
            context(controller.clone()),
            call(
                "create_goal",
                json!({ "objective": "Original goal", "token_budget": 100 }),
            ),
        )
        .await
        .unwrap();
    assert!(!original.is_error);

    let duplicate = create
        .execute(
            context(controller),
            call(
                "create_goal",
                json!({ "objective": "Replacement goal", "token_budget": 200 }),
            ),
        )
        .await
        .unwrap();

    assert!(duplicate.is_error, "{duplicate:?}");
    assert!(
        duplicate
            .text
            .contains("cannot create a new goal because this thread has an unfinished goal"),
        "{duplicate:?}"
    );
}

#[tokio::test]
async fn create_goal_replaces_completed_goal() {
    let controller = Arc::new(FakeGoalController::default());
    let create = CreateGoalTool;
    let update = UpdateGoalTool;

    let original = create
        .execute(
            context(controller.clone()),
            call("create_goal", json!({ "objective": "Finish phase 111" })),
        )
        .await
        .unwrap();
    assert!(!original.is_error);

    let completed = update
        .execute(
            context(controller.clone()),
            call("update_goal", json!({ "status": "complete" })),
        )
        .await
        .unwrap();
    assert!(!completed.is_error);
    assert_eq!(completed.data["hasActiveGoal"], false);
    assert_eq!(completed.data["goal"]["status"], "complete");

    // Resume-session shape: completed goal is still stored on disk, but a
    // new create_goal must succeed so the harness can start the next job.
    let next = create
        .execute(
            context(controller.clone()),
            call(
                "create_goal",
                json!({ "objective": "Ship the release", "token_budget": 24000 }),
            ),
        )
        .await
        .unwrap();
    assert!(!next.is_error, "create after complete failed: {next:?}");
    assert_eq!(next.data["hasActiveGoal"], true);
    assert_eq!(next.data["goal"]["status"], "active");
    assert_eq!(next.data["goal"]["objective"], "Ship the release");
    assert_eq!(next.data["goal"]["tokenBudget"], 24000);
    assert_eq!(next.data["goal"]["tokensUsed"], 0);

    let stored = controller
        .get_thread_goal(&"thread-goals".to_string())
        .await
        .unwrap()
        .expect("replacement goal should be stored");
    assert_eq!(stored.objective, "Ship the release");
    assert_eq!(stored.status, ThreadGoalStatus::Active);
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments,
        raw_arguments: "{}".to_string(),
        thread_id: "thread-goals".to_string(),
        turn_id: "turn-goals".to_string(),
    }
}

fn context(controller: Arc<dyn ThreadGoalController>) -> ToolExecutionContext {
    ToolExecutionContext::new(
        ThreadId::from("thread-goals"),
        TurnId::from("turn-goals"),
        PolicyMode::Default,
    )
    .with_goal_controller(controller)
}
#[tokio::test]
async fn create_goal_rejects_every_unfinished_status() {
    for status in [
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::UsageLimited,
        ThreadGoalStatus::BudgetLimited,
    ] {
        let controller = Arc::new(FakeGoalController::default());
        CreateGoalTool
            .execute(
                context(controller.clone()),
                call("create_goal", json!({"objective":"Original"})),
            )
            .await
            .unwrap();
        controller.goal.lock().await.as_mut().unwrap().status = status;
        let rejected = CreateGoalTool
            .execute(
                context(controller.clone()),
                call("create_goal", json!({"objective":"Replacement"})),
            )
            .await
            .unwrap();
        assert!(rejected.is_error, "{status:?}");
        assert_eq!(
            controller.goal.lock().await.as_ref().unwrap().objective,
            "Original"
        );
    }
}

#[tokio::test]
async fn model_can_pause_but_cannot_resume_or_change_limits() {
    let controller = Arc::new(FakeGoalController::default());
    CreateGoalTool
        .execute(
            context(controller.clone()),
            call("create_goal", json!({"objective":"Work"})),
        )
        .await
        .unwrap();
    let paused = UpdateGoalTool
        .execute(
            context(controller.clone()),
            call("update_goal", json!({"status":"paused"})),
        )
        .await
        .unwrap();
    assert_eq!(paused.data["goal"]["status"], "paused");
    for status in ["active", "budget_limited", "usage_limited"] {
        assert!(
            UpdateGoalTool
                .execute(
                    context(controller.clone()),
                    call("update_goal", json!({"status":status}))
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn completion_guidance_is_only_returned_by_budgeted_completion() {
    let controller = Arc::new(FakeGoalController::default());
    CreateGoalTool
        .execute(
            context(controller.clone()),
            call(
                "create_goal",
                json!({"objective":"Work", "token_budget":100}),
            ),
        )
        .await
        .unwrap();
    controller.goal.lock().await.as_mut().unwrap().tokens_used = 42;
    let completed = UpdateGoalTool
        .execute(
            context(controller.clone()),
            call("update_goal", json!({"status":"complete"})),
        )
        .await
        .unwrap();
    assert_eq!(completed.data["remainingTokens"], 58);
    assert!(completed.data["completionBudgetReport"].is_string());
    assert!(completed.text.contains("42/100 tokens"));
    let read = GetGoalTool
        .execute(context(controller), call("get_goal", json!({})))
        .await
        .unwrap();
    assert!(read.data["completionBudgetReport"].is_null());
}
