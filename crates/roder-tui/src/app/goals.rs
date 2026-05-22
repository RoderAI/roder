use roder_app_server::AppClient;
use roder_protocol::{
    JsonRpcRequest, ThreadGoal, ThreadGoalClearParams, ThreadGoalClearResult, ThreadGoalGetParams,
    ThreadGoalGetResult, ThreadGoalSetParams, ThreadGoalSetResult, ThreadGoalStatus,
};

use super::{TuiApp, composer_textarea, decode_response, slash_command_suffix, truncate};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_goal_slash_command(&mut self, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            match thread_goal_get(&self.client, &self.thread_id).await {
                Ok(result) => {
                    self.current_goal = result.goal;
                    self.timeline
                        .push_system(goal_summary(self.current_goal.as_ref()));
                }
                Err(err) => self.record_error(format!("thread/goal/get failed: {err}")),
            }
            self.push_event("slash command: /goal".to_string());
            return;
        }

        let (action, rest) = split_goal_action(args);
        match action {
            "clear" if rest.is_empty() => {
                match thread_goal_clear(&self.client, &self.thread_id).await {
                    Ok(result) => {
                        self.current_goal = None;
                        let text = if result.cleared {
                            "Goal cleared.".to_string()
                        } else {
                            "No goal to clear.".to_string()
                        };
                        self.timeline.push_system(text);
                    }
                    Err(err) => self.record_error(format!("thread/goal/clear failed: {err}")),
                }
            }
            "pause" if rest.is_empty() => {
                self.set_goal_status(ThreadGoalStatus::Paused, "pause")
                    .await;
            }
            "resume" if rest.is_empty() => {
                self.set_goal_status(ThreadGoalStatus::Active, "resume")
                    .await;
            }
            "edit" => {
                self.edit_goal(rest).await;
            }
            _ => {
                self.set_goal_objective(args).await;
            }
        }
        self.push_event(format!(
            "slash command: /goal{}",
            slash_command_suffix(args)
        ));
    }

    async fn set_goal_status(&mut self, status: ThreadGoalStatus, action: &str) {
        match thread_goal_set(
            &self.client,
            ThreadGoalSetParams {
                thread_id: self.thread_id.clone(),
                objective: None,
                status: Some(status),
                token_budget: None,
            },
        )
        .await
        {
            Ok(result) => {
                self.current_goal = result.goal;
                self.timeline
                    .push_system(goal_summary(self.current_goal.as_ref()));
            }
            Err(err) => self.record_error(format!("thread/goal/{action} failed: {err}")),
        }
    }

    async fn edit_goal(&mut self, objective: &str) {
        if objective.trim().is_empty() {
            match thread_goal_get(&self.client, &self.thread_id).await {
                Ok(result) => {
                    self.current_goal = result.goal;
                    if let Some(goal) = &self.current_goal {
                        self.composer = composer_textarea(self.theme);
                        self.composer
                            .insert_str(&format!("/goal edit {}", goal.objective));
                        self.timeline.push_system("Editing goal objective.");
                    } else {
                        self.timeline.push_system("No goal to edit.");
                    }
                }
                Err(err) => self.record_error(format!("thread/goal/get failed: {err}")),
            }
            return;
        }
        self.set_goal_objective(objective).await;
    }

    async fn set_goal_objective(&mut self, objective: &str) {
        match thread_goal_set(
            &self.client,
            ThreadGoalSetParams {
                thread_id: self.thread_id.clone(),
                objective: Some(objective.trim().to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: None,
            },
        )
        .await
        {
            Ok(result) => {
                self.current_goal = result.goal;
                self.timeline
                    .push_system(goal_summary(self.current_goal.as_ref()));
            }
            Err(err) => self.record_error(format!("thread/goal/set failed: {err}")),
        }
    }
}

pub(super) async fn thread_goal_get<C: AppClient>(
    client: &C,
    thread_id: &str,
) -> anyhow::Result<ThreadGoalGetResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/goal/get")),
            method: "thread/goal/get".to_string(),
            params: Some(serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread_id.to_string(),
            })?),
        })
        .await;
    decode_response(res)
}

async fn thread_goal_set<C: AppClient>(
    client: &C,
    params: ThreadGoalSetParams,
) -> anyhow::Result<ThreadGoalSetResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/goal/set")),
            method: "thread/goal/set".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(res)
}

async fn thread_goal_clear<C: AppClient>(
    client: &C,
    thread_id: &str,
) -> anyhow::Result<ThreadGoalClearResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/goal/clear")),
            method: "thread/goal/clear".to_string(),
            params: Some(serde_json::to_value(ThreadGoalClearParams {
                thread_id: thread_id.to_string(),
            })?),
        })
        .await;
    decode_response(res)
}

pub(super) fn goal_footer_label(goal: &ThreadGoal) -> String {
    format!("{}:{}", goal.status.as_str(), truncate(&goal.objective, 28))
}

fn goal_summary(goal: Option<&ThreadGoal>) -> String {
    let Some(goal) = goal else {
        return "No goal set.".to_string();
    };
    let budget = match goal.token_budget {
        Some(budget) => format!("{}/{} tokens", goal.tokens_used, budget),
        None => format!("{} tokens", goal.tokens_used),
    };
    format!(
        "Goal {}: {}\nUsage: {}, {}s elapsed.\nCommands: /goal pause, /goal resume, /goal edit, /goal clear.",
        goal.status.as_str(),
        goal.objective,
        budget,
        goal.time_used_seconds
    )
}

fn split_goal_action(args: &str) -> (&str, &str) {
    let mut parts = args.splitn(2, char::is_whitespace);
    let action = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default().trim();
    (action, rest)
}
