use roder_app_server::AppClient;
use roder_protocol::{
    JsonRpcRequest, ThreadGoal, ThreadGoalClearParams, ThreadGoalClearResult, ThreadGoalGetParams,
    ThreadGoalGetResult, ThreadGoalSetParams, ThreadGoalSetResult, ThreadGoalStatus,
};

use super::{
    ConfirmDialog, ConfirmDialogState, TuiApp, composer_textarea, decode_response,
    slash_command_suffix, truncate,
};

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
                            .insert_str(format!("/goal edit {}", goal.objective));
                        self.timeline.push_system("Editing goal objective.");
                    } else {
                        self.timeline.push_system("No goal to edit.");
                    }
                }
                Err(err) => self.record_error(format!("thread/goal/get failed: {err}")),
            }
            return;
        }
        match thread_goal_get(&self.client, &self.thread_id).await {
            Ok(result) => {
                self.current_goal = result.goal;
                let Some(goal) = self.current_goal.as_ref() else {
                    self.timeline.push_system("No goal to edit.");
                    return;
                };
                let status = edited_goal_status(goal.status);
                self.update_goal_objective(objective, status).await;
            }
            Err(err) => self.record_error(format!("thread/goal/get failed: {err}")),
        }
    }

    async fn set_goal_objective(&mut self, objective: &str) {
        match thread_goal_get(&self.client, &self.thread_id).await {
            Ok(result) => {
                self.current_goal = result.goal;
                if self
                    .current_goal
                    .as_ref()
                    .is_some_and(|goal| goal.status != ThreadGoalStatus::Complete)
                {
                    self.confirm_dialog =
                        Some(ConfirmDialogState::new(ConfirmDialog::ReplaceGoal {
                            objective: objective.to_string(),
                        }));
                    return;
                }
            }
            Err(err) => {
                self.record_error(format!("thread/goal/get failed: {err}"));
                return;
            }
        }
        self.replace_goal_objective(objective).await;
    }

    pub(super) async fn replace_goal_objective(&mut self, objective: &str) {
        // Validate before clearing so an invalid replacement cannot discard the old goal.
        if let Err(err) = roder_api::goals::validate_thread_goal_objective(objective) {
            self.record_error(err.to_string());
            return;
        }
        if self.current_goal.is_some() {
            match thread_goal_clear(&self.client, &self.thread_id).await {
                Ok(_) => self.current_goal = None,
                Err(err) => {
                    self.record_error(format!("thread/goal/replace failed: {err}"));
                    return;
                }
            }
        }
        self.update_goal_objective(objective, ThreadGoalStatus::Active)
            .await;
    }

    async fn update_goal_objective(&mut self, objective: &str, status: ThreadGoalStatus) {
        match thread_goal_set(
            &self.client,
            ThreadGoalSetParams {
                thread_id: self.thread_id.clone(),
                objective: Some(objective.trim().to_string()),
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
    let commands = match goal.status {
        ThreadGoalStatus::Active => "/goal edit, /goal pause, /goal clear",
        ThreadGoalStatus::Paused | ThreadGoalStatus::Blocked | ThreadGoalStatus::UsageLimited => {
            "/goal edit, /goal resume, /goal clear"
        }
        ThreadGoalStatus::BudgetLimited | ThreadGoalStatus::Complete => "/goal edit, /goal clear",
    };
    format!(
        "Goal {}: {}\nUsage: {}, {}s elapsed.\nCommands: {}.",
        goal.status.as_str(),
        goal.objective,
        budget,
        goal.time_used_seconds,
        commands
    )
}

fn split_goal_action(args: &str) -> (&str, &str) {
    let mut parts = args.splitn(2, char::is_whitespace);
    let action = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default().trim();
    (action, rest)
}

fn edited_goal_status(status: ThreadGoalStatus) -> ThreadGoalStatus {
    match status {
        ThreadGoalStatus::Complete | ThreadGoalStatus::BudgetLimited => ThreadGoalStatus::Active,
        other => other,
    }
}

#[cfg(test)]
mod tests;
