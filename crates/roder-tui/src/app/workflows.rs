use crossterm::event::{KeyCode, KeyEvent};
use roder_api::dynamic_workflows::WorkflowApprovalDecision;
use roder_protocol::{
    JsonRpcRequest, WorkflowsApproveParams, WorkflowsApproveResult, WorkflowsGetParams,
    WorkflowsGetResult, WorkflowsListParams, WorkflowsListResult, WorkflowsPauseParams,
    WorkflowsPauseResult, WorkflowsPlanParams, WorkflowsPlanResult, WorkflowsRestartAgentParams,
    WorkflowsRestartAgentResult, WorkflowsResumeParams, WorkflowsResumeResult, WorkflowsSaveParams,
    WorkflowsSaveResult, WorkflowsSaveScope, WorkflowsScriptsListParams,
    WorkflowsScriptsListResult, WorkflowsStopParams, WorkflowsStopResult,
};

use super::composer::{composer_text, composer_textarea};
use super::{AppClient, TuiApp, decode_response, short_id};

mod detail;
mod progress;
mod render;
mod state;

#[cfg(test)]
mod tests;

use state::WorkflowUiAction;
pub(super) use state::WorkflowUiState;

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_workflows_slash_command(&mut self, args: &str) {
        let parts = args.split_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            [] | ["list"] => self.show_workflows().await,
            ["scripts"] | ["scripts", "list"] => self.show_workflow_scripts().await,
            ["get", run_id] | [run_id] => self.show_workflow_detail(run_id).await,
            ["pause", run_id] => self.pause_workflow(run_id).await,
            ["resume", run_id] => self.resume_workflow(run_id).await,
            ["stop", run_id] => self.stop_workflow(run_id).await,
            ["save", run_id] => self.save_workflow(run_id).await,
            ["restart-agent", run_id, agent_id] => {
                self.restart_workflow_agent(run_id, agent_id).await;
            }
            _ => {
                self.timeline.push_system(
                    "Usage: /workflows [list|<run-id>|get <run-id>|pause <run-id>|resume <run-id>|stop <run-id>|save <run-id>|restart-agent <run-id> <agent-id>|scripts]",
                );
                self.push_event("slash command: /workflows".to_string());
            }
        }
    }

    pub(super) async fn handle_workflow_key(&mut self, key: KeyEvent) -> bool {
        let Some(action) = self.workflows.key_action(key) else {
            return false;
        };
        match action {
            WorkflowUiAction::Approve(decision) => self.approve_pending_workflow(decision).await,
            WorkflowUiAction::Deny => {
                self.approve_pending_workflow(WorkflowApprovalDecision::Deny)
                    .await
            }
            WorkflowUiAction::EditPrompt => self.edit_pending_workflow_prompt(),
            WorkflowUiAction::ToggleScript => {}
            WorkflowUiAction::Close => self.workflows.close_panel(),
            WorkflowUiAction::Back => self.workflows.back_panel(),
            WorkflowUiAction::RefreshList => self.show_workflows().await,
            WorkflowUiAction::OpenSelected => {
                if let Some(run_id) = self.workflows.selected_run_id() {
                    self.show_workflow_detail(&run_id).await;
                }
            }
            WorkflowUiAction::MoveSelection(delta) => self.workflows.move_selection(delta),
            WorkflowUiAction::PauseSelected => {
                if let Some(run_id) = self.workflows.selected_run_id() {
                    self.pause_workflow(&run_id).await;
                }
            }
            WorkflowUiAction::ResumeSelected => {
                if let Some(run_id) = self.workflows.selected_run_id() {
                    self.resume_workflow(&run_id).await;
                }
            }
            WorkflowUiAction::StopSelected => {
                if let Some(run_id) = self.workflows.selected_run_id() {
                    self.stop_workflow(&run_id).await;
                }
            }
            WorkflowUiAction::SaveSelected => {
                if let Some(run_id) = self.workflows.selected_run_id() {
                    self.save_workflow(&run_id).await;
                }
            }
            WorkflowUiAction::RestartSelectedAgent => {
                if let Some((run_id, agent_id)) = self.workflows.selected_agent_id() {
                    self.restart_workflow_agent(&run_id, &agent_id).await;
                }
            }
        }
        true
    }

    pub(super) async fn handle_workflow_trigger_key(&mut self, key: KeyEvent) -> bool {
        let input = composer_text(&self.composer);
        self.workflows.reset_ignored_trigger_if_changed(&input);
        if !self.workflows.trigger_active(&input)
            || key.modifiers != crossterm::event::KeyModifiers::NONE
        {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.workflows.ignore_trigger(&input);
                self.push_event("workflow trigger ignored".to_string());
                true
            }
            KeyCode::Enter => {
                self.plan_workflow_from_prompt(input.trim().to_string())
                    .await;
                true
            }
            _ => false,
        }
    }

    pub(super) async fn maybe_plan_workflow_from_composer(&mut self) -> bool {
        let input = composer_text(&self.composer);
        self.workflows.reset_ignored_trigger_if_changed(&input);
        if self.active_turn_id.is_some()
            || !self.image_attachments.is_empty()
            || !self.workflows.trigger_active(&input)
        {
            return false;
        }
        self.plan_workflow_from_prompt(input.trim().to_string())
            .await;
        true
    }

    async fn plan_workflow_from_prompt(&mut self, prompt: String) {
        self.composer = composer_textarea(self.theme);
        let result = self
            .workflow_request::<WorkflowsPlanResult, _>(
                "workflows/plan",
                WorkflowsPlanParams {
                    thread_id: Some(self.thread_id.clone()),
                    turn_id: None,
                    prompt: prompt.clone(),
                    workspace: None,
                    arguments: serde_json::json!({}),
                    script: None,
                },
            )
            .await;
        match result {
            Ok(result) => {
                let approval_id = format!("approval-{}", result.run.run_id);
                self.workflows
                    .start_approval(approval_id, result.run.clone(), prompt);
                self.timeline.push_system(format!(
                    "Workflow drafted: {} ({})",
                    result.run.script.name,
                    short_id(&result.run.run_id)
                ));
                self.push_event("workflow plan drafted".to_string());
            }
            Err(err) => {
                self.record_error(format!("workflows/plan failed: {err}"));
                self.composer.insert_str(prompt);
            }
        }
    }

    async fn approve_pending_workflow(&mut self, decision: WorkflowApprovalDecision) {
        let Some(run_id) = self.workflows.approval_run_id() else {
            return;
        };
        let result = self
            .workflow_request::<WorkflowsApproveResult, _>(
                "workflows/approve",
                WorkflowsApproveParams {
                    run_id: run_id.clone(),
                    decision,
                    reason: Some("TUI workflow approval".to_string()),
                },
            )
            .await;
        match result {
            Ok(result) => {
                self.workflows.clear_approval();
                self.workflows.record_run(result.run.clone());
                if result.approval.decision == WorkflowApprovalDecision::Deny {
                    self.timeline
                        .push_system(format!("Workflow denied: {}", short_id(&run_id)));
                } else {
                    self.timeline
                        .push_system(format!("Workflow started: {}", short_id(&run_id)));
                }
                self.push_event(format!("workflow approval: {:?}", result.approval.decision));
            }
            Err(err) => self.record_error(format!("workflows/approve failed: {err}")),
        }
    }

    fn edit_pending_workflow_prompt(&mut self) {
        if let Some(prompt) = self.workflows.approval_prompt() {
            self.workflows.clear_approval();
            self.composer = composer_textarea(self.theme);
            self.composer.insert_str(prompt);
            self.timeline.focus_composer();
            self.push_event("workflow prompt restored".to_string());
        }
    }

    async fn show_workflows(&mut self) {
        match self
            .workflow_request::<WorkflowsListResult, _>(
                "workflows/list",
                WorkflowsListParams {
                    thread_id: None,
                    include_terminal: true,
                },
            )
            .await
        {
            Ok(result) => {
                self.workflows.set_list(result.runs);
                self.push_event("slash command: /workflows".to_string());
            }
            Err(err) => self.record_error(format!("workflows/list failed: {err}")),
        }
    }

    async fn show_workflow_detail(&mut self, run_id: &str) {
        match self
            .workflow_request::<WorkflowsGetResult, _>(
                "workflows/get",
                WorkflowsGetParams {
                    run_id: run_id.to_string(),
                    include_script_body: true,
                    include_agents: true,
                },
            )
            .await
        {
            Ok(result) => {
                self.workflows.show_detail(result.run);
                self.push_event(format!("workflow detail: {}", short_id(run_id)));
            }
            Err(err) => self.record_error(format!("workflows/get failed: {err}")),
        }
    }

    async fn pause_workflow(&mut self, run_id: &str) {
        match self
            .workflow_request::<WorkflowsPauseResult, _>(
                "workflows/pause",
                WorkflowsPauseParams {
                    run_id: run_id.to_string(),
                    cancel_running_agents: false,
                    reason: Some("TUI pause".to_string()),
                },
            )
            .await
        {
            Ok(result) => self.workflows.record_run(result.run),
            Err(err) => self.record_error(format!("workflows/pause failed: {err}")),
        }
    }

    async fn resume_workflow(&mut self, run_id: &str) {
        match self
            .workflow_request::<WorkflowsResumeResult, _>(
                "workflows/resume",
                WorkflowsResumeParams {
                    run_id: run_id.to_string(),
                },
            )
            .await
        {
            Ok(result) => self.workflows.record_run(result.run),
            Err(err) => self.record_error(format!("workflows/resume failed: {err}")),
        }
    }

    async fn stop_workflow(&mut self, run_id: &str) {
        match self
            .workflow_request::<WorkflowsStopResult, _>(
                "workflows/stop",
                WorkflowsStopParams {
                    run_id: run_id.to_string(),
                    reason: Some("TUI stop".to_string()),
                },
            )
            .await
        {
            Ok(result) => self.workflows.record_run(result.run),
            Err(err) => self.record_error(format!("workflows/stop failed: {err}")),
        }
    }

    async fn save_workflow(&mut self, run_id: &str) {
        let run = match self.cached_or_loaded_workflow(run_id).await {
            Ok(run) => run,
            Err(err) => {
                self.record_error(err);
                return;
            }
        };
        match self
            .workflow_request::<WorkflowsSaveResult, _>(
                "workflows/save",
                WorkflowsSaveParams {
                    run_id: run_id.to_string(),
                    name: run.script.name.clone(),
                    scope: WorkflowsSaveScope::Workspace,
                    overwrite: true,
                },
            )
            .await
        {
            Ok(result) => {
                self.timeline.push_system(format!(
                    "Workflow script saved: {}",
                    result
                        .script
                        .source
                        .path
                        .as_deref()
                        .unwrap_or(&result.script.name)
                ));
                self.push_event(format!("workflow saved: {}", short_id(run_id)));
            }
            Err(err) => self.record_error(format!("workflows/save failed: {err}")),
        }
    }

    async fn cached_or_loaded_workflow(
        &mut self,
        run_id: &str,
    ) -> Result<roder_api::dynamic_workflows::WorkflowRun, String> {
        if let Some(run) = self.workflows.cached_run(run_id) {
            return Ok(run);
        }
        match self
            .workflow_request::<WorkflowsGetResult, _>(
                "workflows/get",
                WorkflowsGetParams {
                    run_id: run_id.to_string(),
                    include_script_body: true,
                    include_agents: true,
                },
            )
            .await
        {
            Ok(result) => {
                self.workflows.record_run(result.run.clone());
                Ok(result.run)
            }
            Err(err) => Err(format!("workflows/get failed before save: {err}")),
        }
    }

    async fn restart_workflow_agent(&mut self, run_id: &str, agent_id: &str) {
        match self
            .workflow_request::<WorkflowsRestartAgentResult, _>(
                "workflows/restartAgent",
                WorkflowsRestartAgentParams {
                    run_id: run_id.to_string(),
                    agent_id: agent_id.to_string(),
                },
            )
            .await
        {
            Ok(result) => self.workflows.record_run(result.run),
            Err(err) => self.record_error(format!("workflows/restartAgent failed: {err}")),
        }
    }

    async fn show_workflow_scripts(&mut self) {
        match self
            .workflow_request::<WorkflowsScriptsListResult, _>(
                "workflows/scripts/list",
                WorkflowsScriptsListParams {
                    workspace: None,
                    include_user: true,
                    include_builtin: true,
                },
            )
            .await
        {
            Ok(result) if result.scripts.is_empty() => {
                self.timeline.push_system("No saved workflow scripts.");
            }
            Ok(result) => {
                let mut lines = vec!["Workflow scripts:".to_string()];
                lines.extend(result.scripts.iter().map(|script| {
                    format!(
                        "/{} {}",
                        script.name,
                        script.description.as_deref().unwrap_or("workflow")
                    )
                }));
                self.timeline.push_system(lines.join("\n"));
            }
            Err(err) => self.record_error(format!("workflows/scripts/list failed: {err}")),
        }
    }

    async fn workflow_request<T, P>(&self, method: &str, params: P) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
        P: serde::Serialize,
    {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(method)),
                method: method.to_string(),
                params: Some(serde_json::to_value(params)?),
            })
            .await;
        decode_response(res)
    }
}
