use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::files::{parse, require_nonempty, result};

pub(crate) fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    let plan_state = Arc::new(Mutex::new(PlanState::default()));
    let goal_state = Arc::new(Mutex::new(GoalState::default()));

    registry.register(Arc::new(UpdatePlanTool { state: plan_state }))?;
    registry.register(Arc::new(GetGoalTool {
        state: goal_state.clone(),
    }))?;
    registry.register(Arc::new(CreateGoalTool {
        state: goal_state.clone(),
    }))?;
    registry.register(Arc::new(UpdateGoalTool { state: goal_state }))?;
    registry.register(Arc::new(RequestUserInputTool))
}

#[derive(Debug, Default)]
struct PlanState {
    explanation: Option<String>,
    items: Vec<PlanItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlanItem {
    step: String,
    status: PlanStatus,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Default)]
struct GoalState {
    active: Option<Goal>,
}

#[derive(Debug, Clone)]
struct Goal {
    objective: String,
    token_budget: Option<u64>,
    status: GoalStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalStatus {
    Active,
    Complete,
}

#[derive(Debug)]
struct UpdatePlanTool {
    state: Arc<Mutex<PlanState>>,
}

#[derive(Debug)]
struct GetGoalTool {
    state: Arc<Mutex<GoalState>>,
}

#[derive(Debug)]
struct CreateGoalTool {
    state: Arc<Mutex<GoalState>>,
}

#[derive(Debug)]
struct UpdateGoalTool {
    state: Arc<Mutex<GoalState>>,
}

#[derive(Debug)]
struct RequestUserInputTool;

#[async_trait::async_trait]
impl ToolExecutor for UpdatePlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "update_plan".to_string(),
            description: "Updates the task plan.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "explanation": {
                        "type": "string",
                        "description": "Optional explanation for the plan update."
                    },
                    "plan": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["step", "status"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["plan"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<UpdatePlanArgs>(&call)?;
        let in_progress = args
            .plan
            .iter()
            .filter(|item| item.status == PlanStatus::InProgress)
            .count();
        if in_progress > 1 {
            return Ok(error_result(
                call,
                "update_plan accepts at most one in_progress item".to_string(),
            ));
        }
        for item in &args.plan {
            require_nonempty(item.step.trim(), "step")?;
        }

        let mut state = self.state.lock().await;
        state.explanation = args.explanation;
        state.items = args.plan;
        let text = format_plan(&state);
        Ok(result(
            call,
            text,
            json!({
                "explanation": state.explanation,
                "plan": state.items.iter().map(plan_item_json).collect::<Vec<_>>(),
            }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GetGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "get_goal".to_string(),
            description: "Get the current goal for this thread.".to_string(),
            parameters: empty_params(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let state = self.state.lock().await;
        let data = goal_state_json(&state);
        Ok(result(call, goal_text(&data), data, false))
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
                    "objective": { "type": "string" },
                    "token_budget": {
                        "type": "integer",
                        "minimum": 1
                    },
                    "replace": {
                        "type": "boolean",
                        "description": "Replace an existing active goal when true."
                    }
                },
                "required": ["objective"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<CreateGoalArgs>(&call)?;
        let objective = args.objective.trim().to_string();
        require_nonempty(&objective, "objective")?;
        let mut state = self.state.lock().await;
        if matches!(
            state.active.as_ref().map(|goal| goal.status),
            Some(GoalStatus::Active)
        ) && !args.replace.unwrap_or(false)
        {
            return Ok(error_result(
                call,
                "an active goal already exists".to_string(),
            ));
        }
        state.active = Some(Goal {
            objective,
            token_budget: args.token_budget,
            status: GoalStatus::Active,
        });
        let data = goal_state_json(&state);
        Ok(result(call, goal_text(&data), data, false))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for UpdateGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "update_goal".to_string(),
            description: "Update the existing goal. Only status=complete is supported.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["complete"]
                    }
                },
                "required": ["status"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<UpdateGoalArgs>(&call)?;
        if args.status != "complete" {
            return Ok(error_result(
                call,
                "update_goal only accepts status=complete".to_string(),
            ));
        }
        let mut state = self.state.lock().await;
        let Some(goal) = state.active.as_mut() else {
            return Ok(error_result(call, "no active goal exists".to_string()));
        };
        goal.status = GoalStatus::Complete;
        let data = goal_state_json(&state);
        Ok(result(call, goal_text(&data), data, false))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RequestUserInputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "request_user_input".to_string(),
            description:
                "Request user input for one to three short questions and wait for the response."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 3,
                        "items": {
                            "type": "object",
                            "properties": {
                                "header": { "type": "string" },
                                "id": { "type": "string" },
                                "question": { "type": "string" },
                                "options": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": { "type": "string" },
                                            "description": { "type": "string" }
                                        },
                                        "required": ["label", "description"],
                                        "additionalProperties": false
                                    }
                                }
                            },
                            "required": ["header", "id", "question", "options"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["questions"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<RequestUserInputArgs>(&call)?;
        if args.questions.is_empty() || args.questions.len() > 3 {
            return Ok(error_result(
                call,
                "request_user_input requires one to three questions".to_string(),
            ));
        }
        for question in &args.questions {
            require_nonempty(question.header.trim(), "header")?;
            require_nonempty(question.id.trim(), "id")?;
            require_nonempty(question.question.trim(), "question")?;
            if question.options.len() < 2 || question.options.len() > 3 {
                return Ok(error_result(
                    call,
                    "each request_user_input question requires two or three options".to_string(),
                ));
            }
            for option in &question.options {
                require_nonempty(option.label.trim(), "label")?;
                require_nonempty(option.description.trim(), "description")?;
            }
        }
        let request = json!({
            "request_id": call.id,
            "questions": args.questions.iter().map(user_question_json).collect::<Vec<_>>(),
        });
        Ok(result(
            call,
            "waiting for user input".to_string(),
            json!({ "user_input_request": request }),
            false,
        ))
    }
}

#[derive(Deserialize)]
struct UpdatePlanArgs {
    explanation: Option<String>,
    plan: Vec<PlanItem>,
}

#[derive(Deserialize)]
struct CreateGoalArgs {
    objective: String,
    token_budget: Option<u64>,
    replace: Option<bool>,
}

#[derive(Deserialize)]
struct UpdateGoalArgs {
    status: String,
}

#[derive(Deserialize)]
struct RequestUserInputArgs {
    questions: Vec<UserQuestion>,
}

#[derive(Deserialize)]
struct UserQuestion {
    header: String,
    id: String,
    question: String,
    options: Vec<UserInputOption>,
}

#[derive(Deserialize)]
struct UserInputOption {
    label: String,
    description: String,
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

fn format_plan(state: &PlanState) -> String {
    let mut text = String::new();
    if let Some(explanation) = &state.explanation
        && !explanation.trim().is_empty()
    {
        text.push_str(explanation.trim());
        text.push('\n');
    }
    for item in &state.items {
        text.push_str("- ");
        text.push_str(status_label(&item.status));
        text.push_str(": ");
        text.push_str(item.step.trim());
        text.push('\n');
    }
    text.trim_end().to_string()
}

fn plan_item_json(item: &PlanItem) -> Value {
    json!({
        "step": item.step,
        "status": status_label(&item.status),
    })
}

fn status_label(status: &PlanStatus) -> &'static str {
    match status {
        PlanStatus::Pending => "pending",
        PlanStatus::InProgress => "in_progress",
        PlanStatus::Completed => "completed",
    }
}

fn goal_state_json(state: &GoalState) -> Value {
    let goal = state.active.as_ref().map(|goal| {
        json!({
            "objective": goal.objective,
            "status": match goal.status {
                GoalStatus::Active => "active",
                GoalStatus::Complete => "complete",
            },
            "token_budget": goal.token_budget,
        })
    });
    json!({
        "goal": goal,
        "has_active_goal": matches!(
            state.active.as_ref().map(|goal| goal.status),
            Some(GoalStatus::Active)
        ),
    })
}

fn goal_text(data: &Value) -> String {
    match data.get("goal") {
        Some(Value::Object(goal)) => {
            let objective = goal
                .get("objective")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = goal
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("Goal {status}: {objective}")
        }
        _ => "No active goal.".to_string(),
    }
}

fn user_question_json(question: &UserQuestion) -> Value {
    json!({
        "header": question.header,
        "id": question.id,
        "question": question.question,
        "options": question.options.iter().map(user_option_json).collect::<Vec<_>>(),
    })
}

fn user_option_json(option: &UserInputOption) -> Value {
    json!({
        "label": option.label,
        "description": option.description,
    })
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::policy_mode::PolicyMode;

    use super::*;

    #[tokio::test]
    async fn update_plan_rejects_multiple_in_progress_items() {
        let tool = UpdatePlanTool {
            state: Arc::new(Mutex::new(PlanState::default())),
        };

        let result = tool
            .execute(
                context(),
                call(
                    "update_plan",
                    json!({
                        "plan": [
                            { "step": "one", "status": "in_progress" },
                            { "step": "two", "status": "in_progress" }
                        ]
                    }),
                ),
            )
            .await
            .unwrap();

        assert!(result.is_error);
    }

    #[tokio::test]
    async fn goal_tools_create_get_and_complete_goal() {
        let state = Arc::new(Mutex::new(GoalState::default()));
        let create = CreateGoalTool {
            state: state.clone(),
        };
        let get = GetGoalTool {
            state: state.clone(),
        };
        let update = UpdateGoalTool { state };

        let created = create
            .execute(
                context(),
                call("create_goal", json!({ "objective": "Ship parity" })),
            )
            .await
            .unwrap();
        assert!(!created.is_error);
        assert_eq!(created.data["has_active_goal"], true);

        let current = get
            .execute(context(), call("get_goal", json!({})))
            .await
            .unwrap();
        assert!(current.text.contains("Ship parity"));

        let completed = update
            .execute(
                context(),
                call("update_goal", json!({ "status": "complete" })),
            )
            .await
            .unwrap();
        assert!(!completed.is_error);
        assert_eq!(completed.data["has_active_goal"], false);
    }

    #[tokio::test]
    async fn request_user_input_returns_pending_request_payload() {
        let tool = RequestUserInputTool;

        let result = tool
            .execute(
                context(),
                call(
                    "request_user_input",
                    json!({
                        "questions": [{
                            "header": "Mode",
                            "id": "mode",
                            "question": "Which mode?",
                            "options": [
                                { "label": "Safe", "description": "Keep restrictions." },
                                { "label": "Fast", "description": "Allow more automation." }
                            ]
                        }]
                    }),
                ),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(
            result.data["user_input_request"]["questions"][0]["id"],
            "mode"
        );
    }

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments,
            raw_arguments: "{}".to_string(),
            thread_id: "thread-workflow".to_string(),
            turn_id: "turn-workflow".to_string(),
        }
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext {
            thread_id: ThreadId::from("thread-workflow"),
            turn_id: TurnId::from("turn-workflow"),
            effective_mode: PolicyMode::Default,
        }
    }
}
