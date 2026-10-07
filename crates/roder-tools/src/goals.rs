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
            goal_output_text(goal.as_ref(), false),
            goal_data(goal.as_ref(), false),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for CreateGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "create_goal".to_string(),
            description:
                "Create a goal only when explicitly requested by the user or system/developer instructions; do not infer goals from ordinary tasks. Set token_budget only when an explicit token budget is requested. Fails if an unfinished goal exists; use update_goal only for status."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "objective": {
                        "type": "string",
                        "description": "The concrete objective to start pursuing. Starts a new active goal when no goal exists or replaces the current goal when it is complete."
                    },
                    "token_budget": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Positive token budget for the new goal. Omit unless explicitly requested."
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
        if let Some(existing) = controller.get_thread_goal(&ctx.thread_id).await?
            && existing.status != ThreadGoalStatus::Complete
        {
            return Ok(error_result(call,
                "cannot create a new goal because this thread has an unfinished goal; complete the existing goal first".to_string()));
        }
        let goal = match controller
            .create_thread_goal(&ctx.thread_id, args.objective, args.token_budget)
            .await
        {
            Ok(goal) => goal,
            Err(err) => return Ok(error_result(call, err.to_string())),
        };
        Ok(result(
            call,
            goal_output_text(Some(&goal), false),
            goal_data(Some(&goal), false),
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
                "Update the existing goal. Set paused only at the user's explicit request, report the returned status, and stop goal work; a later resume revokes that request. Budget limits take precedence over pausing. Set complete only when the full objective has actually been achieved and no required work remains. Set blocked only when the same blocking condition has repeated for at least three consecutive goal turns and meaningful progress is impossible without user input or an external state change. Resuming a blocked goal starts a fresh blocked audit. Once that threshold is satisfied, set blocked instead of leaving the goal active. Do not use blocked merely because the work is hard, slow, uncertain, incomplete, or would benefit from clarification. Do not mark complete merely because the budget is nearly exhausted or because you are stopping work. Resume and limit changes are controlled by the user or system. When marking a budgeted goal complete, report final token usage from this tool result."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["complete", "blocked", "paused"]
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
            ModelGoalStatus::Paused => ThreadGoalStatus::Paused,
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
            goal_output_text(Some(&goal), status == ThreadGoalStatus::Complete),
            goal_data(Some(&goal), status == ThreadGoalStatus::Complete),
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
    Paused,
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

fn goal_data(goal: Option<&ThreadGoal>, report_completion: bool) -> Value {
    let remaining_tokens = goal
        .and_then(|goal| {
            goal.token_budget
                .map(|budget| budget.saturating_sub(goal.tokens_used))
        })
        .map(|remaining| remaining.max(0));
    let completion_budget_report = goal.filter(|goal| report_completion
        && goal.status == ThreadGoalStatus::Complete
        && (goal.token_budget.is_some() || goal.time_used_seconds > 0))
        .map(|_| "Goal achieved. Report final usage from this tool result's structured goal fields. If goal.tokenBudget is present, include goal.tokensUsed and goal.tokenBudget. If goal.timeUsedSeconds is greater than zero, summarize elapsed time in a concise human-friendly form.");
    json!({
        "goal": goal,
        "hasActiveGoal": goal.is_some_and(|goal| goal.status == ThreadGoalStatus::Active),
        "remainingTokens": remaining_tokens,
        "completionBudgetReport": completion_budget_report,
    })
}

// The runtime sends ToolResult.text to inference providers. Keep structured goal
// fields and completion guidance there as well as in the native tools/call data.
fn goal_output_text(goal: Option<&ThreadGoal>, report_completion: bool) -> String {
    format!(
        "{}\n{}",
        goal_text(goal),
        goal_data(goal, report_completion)
    )
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
mod tests;
