use roder_api::processes::{ProcessDescriptor, ProcessState};
use roder_protocol::{
    JsonRpcRequest, ProcessesGetParams, ProcessesGetResult, ProcessesListParams,
    ProcessesListResult, ProcessesStopAllParams, ProcessesStopAllResult, ProcessesStopParams,
    ProcessesStopResult,
};

use super::{TuiApp, decode_response, short_id, truncate};

impl TuiApp {
    pub(super) async fn run_processes_slash_command(&mut self, args: &str) {
        let parts = args.split_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            [] => self.show_processes(false).await,
            ["all"] => self.show_processes(true).await,
            ["stop", process_id] => self.stop_process(process_id, Some("slash /ps stop")).await,
            ["stop-all", "--confirm"] => self.stop_all_processes().await,
            ["stop-all"] => {
                self.timeline
                    .push_system("Use /ps stop-all --confirm to stop all Roder-owned processes.");
                self.push_event("slash command: /ps stop-all".to_string());
            }
            [process_id] => self.show_process_detail(process_id).await,
            _ => {
                self.timeline.push_system(
                    "Usage: /ps [all|<process-id>|stop <process-id>|stop-all --confirm]",
                );
                self.push_event("slash command: /ps".to_string());
            }
        }
    }

    pub(super) async fn show_processes(&mut self, include_completed: bool) {
        match self.processes_list(include_completed).await {
            Ok(result) if result.processes.is_empty() => {
                self.timeline.push_system("No Roder-owned processes.");
                self.push_event("slash command: /ps".to_string());
            }
            Ok(result) => {
                let mut lines = vec!["Roder processes:".to_string()];
                lines.extend(result.processes.iter().map(process_row));
                self.timeline.push_system(lines.join("\n"));
                self.push_event(if include_completed {
                    "slash command: /ps all".to_string()
                } else {
                    "slash command: /ps".to_string()
                });
            }
            Err(err) => self.record_error(format!("processes/list failed: {err}")),
        }
    }

    pub(super) async fn show_process_detail(&mut self, process_id: &str) {
        match self.processes_get(process_id, Some(4096)).await {
            Ok(result) => {
                let Some(process) = result.process else {
                    self.timeline
                        .push_system(format!("Process not found: {process_id}"));
                    return;
                };
                let output = result
                    .output
                    .into_iter()
                    .map(|output| output.chunk)
                    .collect::<String>();
                let output = output.trim();
                let output = if output.is_empty() {
                    "No retained output.".to_string()
                } else {
                    truncate(output, 1200)
                };
                self.timeline.push_system(format!(
                    "{}\nOutput:\n{}",
                    process_detail(&process),
                    output
                ));
                self.push_event(format!("slash command: /ps {}", short_id(process_id)));
            }
            Err(err) => self.record_error(format!("processes/get failed: {err}")),
        }
    }

    pub(super) async fn stop_process(&mut self, process_id: &str, reason: Option<&str>) {
        let reason = reason.map(str::to_string);
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("processes/stop")),
                method: "processes/stop".to_string(),
                params: Some(
                    serde_json::to_value(ProcessesStopParams {
                        process_id: process_id.to_string(),
                        reason,
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<ProcessesStopResult>(res) {
            Ok(result) => {
                self.timeline.push_system(format!(
                    "process {} stop requested: {}",
                    short_id(&result.result.process_id),
                    result.result.stopped
                ));
                self.push_event(format!("slash command: /ps stop {}", short_id(process_id)));
            }
            Err(err) => self.record_error(format!("processes/stop failed: {err}")),
        }
    }

    async fn stop_all_processes(&mut self) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("processes/stopAll")),
                method: "processes/stopAll".to_string(),
                params: Some(
                    serde_json::to_value(ProcessesStopAllParams {
                        reason: Some("slash /ps stop-all".to_string()),
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<ProcessesStopAllResult>(res) {
            Ok(result) => {
                let stopped = result
                    .results
                    .iter()
                    .filter(|result| result.stopped)
                    .count();
                self.timeline
                    .push_system(format!("processes stop-all requested: {stopped} stopped"));
                self.push_event("slash command: /ps stop-all --confirm".to_string());
            }
            Err(err) => self.record_error(format!("processes/stopAll failed: {err}")),
        }
    }

    pub(super) async fn processes_list(
        &self,
        include_completed: bool,
    ) -> anyhow::Result<ProcessesListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("processes/list")),
                method: "processes/list".to_string(),
                params: Some(
                    serde_json::to_value(ProcessesListParams { include_completed }).unwrap(),
                ),
            })
            .await;
        decode_response(res)
    }

    pub(super) async fn processes_get(
        &self,
        process_id: &str,
        output_bytes: Option<usize>,
    ) -> anyhow::Result<ProcessesGetResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("processes/get")),
                method: "processes/get".to_string(),
                params: Some(
                    serde_json::to_value(ProcessesGetParams {
                        process_id: process_id.to_string(),
                        output_bytes,
                    })
                    .unwrap(),
                ),
            })
            .await;
        decode_response(res)
    }
}

pub(super) fn process_row(process: &ProcessDescriptor) -> String {
    let state = process_state_label(&process.state);
    let cwd = process.cwd.as_deref().unwrap_or("-");
    let task = process.task_id.as_deref().map(short_id).unwrap_or("-");
    let thread = process.thread_id.as_deref().map(short_id).unwrap_or("-");
    format!(
        "{} {:<8} {:<13} task:{} thread:{} cwd:{} {}",
        short_id(&process.process_id),
        state,
        format!("{:?}", process.origin),
        task,
        thread,
        truncate(cwd, 28),
        truncate(&process.command_summary, 72)
    )
}

fn process_detail(process: &ProcessDescriptor) -> String {
    format!(
        "Process {}\nstate: {}\norigin: {:?}\ncommand: {}\ncwd: {}\ntask: {}\nthread: {}\nturn: {}\nrunner: {}/{}",
        process.process_id,
        process_state_label(&process.state),
        process.origin,
        process.command_summary,
        process.cwd.as_deref().unwrap_or("-"),
        process.task_id.as_deref().unwrap_or("-"),
        process.thread_id.as_deref().unwrap_or("-"),
        process.turn_id.as_deref().unwrap_or("-"),
        process.runner_destination_id.as_deref().unwrap_or("-"),
        process.runner_session_id.as_deref().unwrap_or("-")
    )
}

fn process_state_label(state: &ProcessState) -> &'static str {
    match state {
        ProcessState::Starting => "starting",
        ProcessState::Running => "running",
        ProcessState::Stopping => "stopping",
        ProcessState::Exited { .. } => "exited",
        ProcessState::Failed { .. } => "failed",
        ProcessState::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use roder_api::processes::{ProcessOrigin, ProcessState};
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn ps_row_is_compact_and_contains_process_identity() {
        let process = ProcessDescriptor {
            process_id: "process-1234567890".to_string(),
            origin: ProcessOrigin::CommandExec,
            state: ProcessState::Running,
            command: vec!["sleep".to_string(), "10".to_string()],
            command_summary: "sleep 10".to_string(),
            cwd: Some("/tmp/workspace".to_string()),
            pid: Some(123),
            task_id: Some("task-abcdef".to_string()),
            thread_id: Some("thread-abcdef".to_string()),
            turn_id: None,
            runner_destination_id: None,
            runner_session_id: None,
            stoppable: true,
            started_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            stdout_tail: None,
            stderr_tail: None,
        };

        let row = process_row(&process);

        assert!(row.contains("process-"));
        assert!(row.contains("running"));
        assert!(row.contains("sleep 10"));
        assert!(row.len() < 140);
    }
}
