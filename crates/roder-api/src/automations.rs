use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::policy_mode::PolicyMode;
use crate::tasks::TaskId;

pub type AutomationId = String;
pub type AutomationRunId = String;
pub type AutomationOccurrenceKey = String;
pub type AutomationServerId = String;
pub type AutomationServerRole = String;
pub type AutomationClientId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDefinition {
    pub id: AutomationId,
    pub name: String,
    pub project: AutomationProject,
    pub schedule: AutomationSchedule,
    pub prompt: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_mode: Option<PolicyMode>,
    pub catch_up: CatchUpPolicy,
    pub concurrency: AutomationConcurrencyPolicy,
    pub created_by: AutomationClient,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationProject {
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AutomationSchedule {
    Cron {
        expression: String,
        timezone: String,
    },
    Interval {
        seconds: u64,
    },
    OneShot {
        #[serde(with = "time::serde::rfc3339")]
        run_at: OffsetDateTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CatchUpPolicy {
    RunAllMissed { max_per_tick: u32 },
    RunLatestOnly,
    SkipExpired { grace_seconds: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationConcurrencyPolicy {
    Forbid,
    Allow,
    ReplaceRunning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationClient {
    pub id: AutomationClientId,
    pub kind: AutomationClientKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationClientKind {
    AppServer,
    Desktop,
    Cli,
    Tui,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunState {
    Scheduled,
    Leased,
    Queued,
    Running,
    Completed,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationLeaseRecord {
    pub run_id: AutomationRunId,
    pub automation_id: AutomationId,
    pub occurrence_key: AutomationOccurrenceKey,
    pub server_id: AutomationServerId,
    pub server_role: AutomationServerRole,
    #[serde(with = "time::serde::rfc3339")]
    pub leased_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunSummary {
    pub run_id: AutomationRunId,
    pub automation_id: AutomationId,
    pub occurrence_key: AutomationOccurrenceKey,
    pub state: AutomationRunState,
    #[serde(with = "time::serde::rfc3339")]
    pub scheduled_for: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub queued_at: Option<OffsetDateTime>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<OffsetDateTime>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<AutomationServerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_role: Option<AutomationServerRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationServerDescriptor {
    pub server_id: AutomationServerId,
    pub server_role: AutomationServerRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCreated {
    pub automation: AutomationDefinition,
    pub server: AutomationServerDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationUpdated {
    pub automation: AutomationDefinition,
    pub server: AutomationServerDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDeleted {
    pub automation_id: AutomationId,
    pub server: AutomationServerDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDue {
    pub automation_id: AutomationId,
    pub occurrence_key: AutomationOccurrenceKey,
    #[serde(with = "time::serde::rfc3339")]
    pub scheduled_for: OffsetDateTime,
    pub server: AutomationServerDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationLeased {
    pub lease: AutomationLeaseRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationQueued {
    pub run: AutomationRunSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationStarted {
    pub run: AutomationRunSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCompleted {
    pub run: AutomationRunSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationFailed {
    pub run: AutomationRunSummary,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSkipped {
    pub run: AutomationRunSummary,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationLeaseExpired {
    pub lease: AutomationLeaseRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    #[test]
    fn automation_definition_uses_stable_public_shape() {
        let definition = AutomationDefinition {
            id: "automation-1".to_string(),
            name: "Nightly cleanup".to_string(),
            project: AutomationProject {
                cwd: "/repo".to_string(),
                display_name: Some("repo".to_string()),
            },
            schedule: AutomationSchedule::Cron {
                expression: "0 2 * * *".to_string(),
                timezone: "Europe/London".to_string(),
            },
            prompt: "summarize status".to_string(),
            enabled: true,
            model_provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            policy_mode: Some(PolicyMode::Plan),
            catch_up: CatchUpPolicy::RunAllMissed { max_per_tick: 3 },
            concurrency: AutomationConcurrencyPolicy::Forbid,
            created_by: AutomationClient {
                id: "desktop-main".to_string(),
                kind: AutomationClientKind::Desktop,
            },
            created_at: timestamp(),
            updated_at: timestamp(),
        };

        let value = serde_json::to_value(&definition).unwrap();
        assert_eq!(value["modelProvider"], "codex");
        assert_eq!(value["policyMode"], "plan");
        assert_eq!(value["catchUp"]["runAllMissed"]["maxPerTick"], 3);
        assert_eq!(value["schedule"]["cron"]["timezone"], "Europe/London");
        assert!(value.get("model_provider").is_none());

        let round_trip: AutomationDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, definition);
    }

    #[test]
    fn run_summary_carries_audit_metadata() {
        let run = AutomationRunSummary {
            run_id: "run-1".to_string(),
            automation_id: "automation-1".to_string(),
            occurrence_key: "automation-1:2026-05-21T02:00:00Z".to_string(),
            state: AutomationRunState::Running,
            scheduled_for: timestamp(),
            queued_at: Some(timestamp()),
            started_at: Some(timestamp()),
            finished_at: None,
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            task_id: Some("task-1".to_string()),
            server_id: Some("desktop-main".to_string()),
            server_role: Some("desktop".to_string()),
            exit_code: None,
            error: None,
            skip_reason: None,
        };

        let value = serde_json::to_value(&run).unwrap();
        assert_eq!(value["runId"], "run-1");
        assert_eq!(value["occurrenceKey"], "automation-1:2026-05-21T02:00:00Z");
        assert_eq!(value["serverId"], "desktop-main");
        assert!(value.get("finishedAt").is_none());
    }
}
