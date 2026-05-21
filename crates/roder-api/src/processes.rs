use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::remote_runner::{RemoteRunnerSessionId, RunnerDestinationId};
use crate::tasks::{TaskId, TaskOutputStream};

pub type ProcessId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessOrigin {
    CommandExec,
    BackgroundTask,
    ShellTool,
    RemoteRunner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ProcessState {
    Starting,
    Running,
    Stopping,
    Exited { exit_code: Option<i32> },
    Failed { error: String },
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDescriptor {
    pub process_id: ProcessId,
    pub origin: ProcessOrigin,
    pub state: ProcessState,
    pub command: Vec<String>,
    pub command_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_destination_id: Option<RunnerDestinationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_session_id: Option<RemoteRunnerSessionId>,
    pub stoppable: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_tail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutput {
    pub process_id: ProcessId,
    pub stream: TaskOutputStream,
    pub chunk: String,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStopResult {
    pub process_id: ProcessId,
    pub stopped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStarted {
    pub process: ProcessDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStopping {
    pub process_id: ProcessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExited {
    pub process: ProcessDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStopped {
    pub process: ProcessDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessFailed {
    pub process: ProcessDescriptor,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

pub fn command_summary(command: &[String]) -> String {
    command
        .iter()
        .map(|part| redact_command_part(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_command_part(part: &str) -> String {
    let lower = part.to_ascii_lowercase();
    let secret_like = [
        "token",
        "secret",
        "password",
        "passwd",
        "apikey",
        "api_key",
        "authorization",
        "bearer",
    ];
    if secret_like.iter().any(|needle| lower.contains(needle)) {
        if let Some((key, _)) = part.split_once('=') {
            format!("{key}=<redacted>")
        } else {
            "<redacted>".to_string()
        }
    } else if part.contains(char::is_whitespace) {
        format!("{part:?}")
    } else {
        part.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> ProcessDescriptor {
        ProcessDescriptor {
            process_id: "process-1".to_string(),
            origin: ProcessOrigin::CommandExec,
            state: ProcessState::Running,
            command: vec![
                "curl".to_string(),
                "Authorization=Bearer abc123".to_string(),
                "https://example.test".to_string(),
            ],
            command_summary: "curl Authorization=<redacted> https://example.test".to_string(),
            cwd: Some("/repo".to_string()),
            pid: Some(1234),
            task_id: Some("task-1".to_string()),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            runner_destination_id: None,
            runner_session_id: None,
            stoppable: true,
            started_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            stdout_tail: Some("ready\n".to_string()),
            stderr_tail: None,
        }
    }

    #[test]
    fn process_descriptor_uses_public_process_id_and_camel_case_fields() {
        let descriptor = descriptor();
        let value = serde_json::to_value(&descriptor).unwrap();

        assert_eq!(value["processId"], "process-1");
        assert_eq!(value["pid"], 1234);
        assert!(value.get("process_id").is_none());
        assert_eq!(value["state"], "running");
        assert_eq!(value["commandSummary"], descriptor.command_summary);

        let decoded: ProcessDescriptor = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, descriptor);
    }

    #[test]
    fn process_state_variants_round_trip_with_payloads() {
        let exited = ProcessState::Exited { exit_code: Some(0) };
        let value = serde_json::to_value(&exited).unwrap();
        assert_eq!(value["exited"]["exitCode"], 0);
        assert_eq!(
            serde_json::from_value::<ProcessState>(value).unwrap(),
            exited
        );

        let failed = ProcessState::Failed {
            error: "spawn failed".to_string(),
        };
        let value = serde_json::to_value(&failed).unwrap();
        assert_eq!(value["failed"]["error"], "spawn failed");
        assert_eq!(
            serde_json::from_value::<ProcessState>(value).unwrap(),
            failed
        );
    }

    #[test]
    fn command_summary_redacts_secret_like_arguments() {
        let command = vec![
            "curl".to_string(),
            "API_KEY=abc123".to_string(),
            "--header".to_string(),
            "Authorization: Bearer abc123".to_string(),
            "hello world".to_string(),
        ];

        assert_eq!(
            command_summary(&command),
            "curl API_KEY=<redacted> --header <redacted> \"hello world\""
        );
    }

    #[test]
    fn process_stop_result_round_trips_descriptor() {
        let result = ProcessStopResult {
            process_id: "process-1".to_string(),
            stopped: true,
            process: Some(descriptor()),
        };

        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["processId"], "process-1");
        assert!(value["process"]["pid"].is_number());
        assert_eq!(
            serde_json::from_value::<ProcessStopResult>(value).unwrap(),
            result
        );
    }
}
