use roder_api::goals::{
    ThreadGoal, ThreadGoalPatch, ThreadGoalStatus, validate_thread_goal_objective,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::files::{parse, result};

pub(crate) fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    registry.register(std::sync::Arc::new(GetGoalTool))?;
    registry.register(std::sync::Arc::new(CreateGoalTool))?;
    registry.register(std::sync::Arc::new(UpdateGoalTool))
}

#[derive(Debug)]
struct GetGoalTool;

#[derive(Debug)]
struct CreateGoalTool;

#[derive(Debug)]
struct UpdateGoalTool;

#[async_trait::async_trait]
impl ToolExecutor for GetGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "get_goal".to_string(),
            description:
                "Get the current goal for this thread, including status, usage, and remaining budget."
                    .to_string(),
            parameters: empty_params(),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let controller = ctx.require_goal_controller()?;
        let goal = controller.get_thread_goal(&ctx.thread_id).await?;
        Ok(result(
            call,
            goal_text(goal.as_ref()),
            goal_data(goal.as_ref()),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for CreateGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "create_goal".to_string(),
            description: "Create a new active goal when no goal is currently defined.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "objective": {
                        "type": "string",
                        "description": "The concrete objective to start pursuing."
                    },
                    "token_budget": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional positive token budget for the active goal."
                    }
                },
                "required": ["objective"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<CreateGoalArgs>(&call)?;
        validate_thread_goal_objective(&args.objective)?;
        let controller = ctx.require_goal_controller()?;
        let goal = match controller
            .create_thread_goal(&ctx.thread_id, args.objective, args.token_budget)
            .await
        {
            Ok(goal) => goal,
            Err(err) => return Ok(error_result(call, err.to_string())),
        };
        Ok(result(
            call,
            goal_text(Some(&goal)),
            goal_data(Some(&goal)),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for UpdateGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "update_goal".to_string(),
            description:
                "Mark the existing goal complete or blocked. Pause, resume, limits, and clear are controlled by the user or runtime."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["complete", "blocked"]
                    }
                },
                "required": ["status"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<UpdateGoalArgs>(&call)?;
        let status = match args.status {
            ModelGoalStatus::Complete => ThreadGoalStatus::Complete,
            ModelGoalStatus::Blocked => ThreadGoalStatus::Blocked,
        };
        let controller = ctx.require_goal_controller()?;
        let Some(goal) = controller
            .set_thread_goal(
                &ctx.thread_id,
                ThreadGoalPatch {
                    objective: None,
                    status: Some(status),
                    token_budget: None,
                },
            )
            .await?
        else {
            return Ok(error_result(call, "no active goal exists".to_string()));
        };
        Ok(result(
            call,
            goal_text(Some(&goal)),
            goal_data(Some(&goal)),
            false,
        ))
    }
}

#[derive(Deserialize)]
struct CreateGoalArgs {
    objective: String,
    token_budget: Option<i64>,
}

#[derive(Deserialize)]
struct UpdateGoalArgs {
    status: ModelGoalStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModelGoalStatus {
    Complete,
    Blocked,
}

fn empty_params() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn error_result(call: ToolCall, message: String) -> ToolResult {
    result(
        call,
        message.clone(),
        json!({
            "error": {
                "kind": "invalid_request",
                "message": message,
            }
        }),
        true,
    )
}

fn goal_data(goal: Option<&ThreadGoal>) -> Value {
    let remaining_tokens = goal
        .and_then(|goal| goal.token_budget.map(|budget| budget - goal.tokens_used))
        .map(|remaining| remaining.max(0));
    let completion_budget_report = goal.and_then(|goal| {
        (goal.status == ThreadGoalStatus::Complete).then(|| match goal.token_budget {
            Some(budget) => format!("Used {} of {} goal tokens.", goal.tokens_used, budget),
            None => format!("Used {} goal tokens.", goal.tokens_used),
        })
    });
    json!({
        "goal": goal,
        "hasActiveGoal": goal.is_some_and(|goal| goal.status == ThreadGoalStatus::Active),
        "remainingTokens": remaining_tokens,
        "completionBudgetReport": completion_budget_report,
    })
}

fn goal_text(goal: Option<&ThreadGoal>) -> String {
    let Some(goal) = goal else {
        return "No active goal.".to_string();
    };
    let budget = match goal.token_budget {
        Some(budget) => format!("{}/{} tokens", goal.tokens_used, budget),
        None => format!("{} tokens", goal.tokens_used),
    };
    format!(
        "Goal {}: {}\nUsage: {}, {}s elapsed.",
        goal.status.as_str(),
        goal.objective,
        budget,
        goal.time_used_seconds
    )
}

#[cfg(test)]
mod tests {
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
        async fn get_thread_goal(
            &self,
            _thread_id: &ThreadId,
        ) -> anyhow::Result<Option<ThreadGoal>> {
            Ok(self.goal.lock().await.clone())
        }

        async fn create_thread_goal(
            &self,
            thread_id: &ThreadId,
            objective: String,
            token_budget: Option<i64>,
        ) -> anyhow::Result<ThreadGoal> {
            if self.goal.lock().await.is_some() {
                anyhow::bail!("an active goal already exists");
            }
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
}
