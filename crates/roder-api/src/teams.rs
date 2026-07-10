use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::policy_mode::PolicyMode;

pub type TeamId = String;
pub type TeamMemberId = String;
pub type TeamTaskId = String;
pub type TeamMessageId = String;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentTeamDisplayMode {
    #[default]
    Auto,
    #[serde(alias = "in-process", alias = "inprocess")]
    InProcess,
    Tmux,
    #[serde(alias = "iterm")]
    Iterm2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTeamConfig {
    pub enabled: bool,
    pub display_mode: AgentTeamDisplayMode,
    pub default_teammate_model: Option<String>,
    pub require_plan_approval: bool,
    pub max_teammates: usize,
    pub split_panes: AgentTeamSplitPaneConfig,
}

impl Default for AgentTeamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            display_mode: AgentTeamDisplayMode::Auto,
            default_teammate_model: Some("lead".to_string()),
            require_plan_approval: false,
            max_teammates: 5,
            split_panes: AgentTeamSplitPaneConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTeamSplitPaneConfig {
    pub reuse_existing_tmux_session: bool,
    pub tmux_command: String,
    pub iterm2_command: String,
}

impl Default for AgentTeamSplitPaneConfig {
    fn default() -> Self {
        Self {
            reuse_existing_tmux_session: true,
            tmux_command: "tmux".to_string(),
            iterm2_command: "it2".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberRole {
    Lead,
    Teammate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberStatus {
    Idle,
    Running,
    Blocked,
    Completed,
    Failed,
    Interrupted,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberDescriptor {
    pub id: TeamMemberId,
    pub role: TeamMemberRole,
    pub name: String,
    #[serde(default)]
    pub task_name: Option<String>,
    #[serde(default)]
    pub agent_path: Option<String>,
    pub thread_id: ThreadId,
    #[serde(default)]
    pub parent_thread_id: Option<ThreadId>,
    pub current_turn_id: Option<TurnId>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub policy_mode: PolicyMode,
    pub status: TeamMemberStatus,
    #[serde(default)]
    pub final_message: Option<String>,
    #[serde(default)]
    pub terminal_error: Option<String>,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamMailboxMessage {
    pub id: TeamMessageId,
    pub team_id: TeamId,
    pub from_member_id: Option<TeamMemberId>,
    pub to_member_id: TeamMemberId,
    #[serde(default)]
    pub kind: TeamMailboxMessageKind,
    pub text: String,
    #[serde(default)]
    pub delivered: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TeamMailboxMessageKind {
    #[default]
    Message,
    NewTask,
    FinalAnswer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TeamTaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamTaskDescriptor {
    pub id: TeamTaskId,
    pub title: String,
    pub status: TeamTaskStatus,
    pub assignee_member_id: Option<TeamMemberId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_display_mode_accepts_snake_case_and_hyphenated_values() {
        let snake: AgentTeamDisplayMode = serde_json::from_str("\"in_process\"").unwrap();
        let hyphen: AgentTeamDisplayMode = serde_json::from_str("\"in-process\"").unwrap();

        assert_eq!(snake, AgentTeamDisplayMode::InProcess);
        assert_eq!(hyphen, AgentTeamDisplayMode::InProcess);
        assert_eq!(
            serde_json::to_string(&AgentTeamDisplayMode::Tmux).unwrap(),
            "\"tmux\""
        );
    }

    #[test]
    fn agent_team_config_defaults_to_disabled_auto() {
        let config = AgentTeamConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.display_mode, AgentTeamDisplayMode::Auto);
        assert_eq!(config.max_teammates, 5);
        assert!(config.split_panes.reuse_existing_tmux_session);
    }

    #[test]
    fn legacy_team_state_fields_deserialize_with_safe_defaults() {
        let member: TeamMemberDescriptor = serde_json::from_value(serde_json::json!({
            "id": "member-1",
            "role": "teammate",
            "name": "Reviewer",
            "threadId": "thread-1",
            "currentTurnId": null,
            "modelProvider": "codex",
            "model": "gpt-5.6-sol",
            "policyMode": "default",
            "status": "idle",
            "paneId": null
        }))
        .unwrap();
        assert_eq!(member.task_name, None);
        assert_eq!(member.agent_path, None);
        assert_eq!(member.parent_thread_id, None);
        assert_eq!(member.final_message, None);
        assert_eq!(member.terminal_error, None);

        let message: TeamMailboxMessage = serde_json::from_value(serde_json::json!({
            "id": "message-1",
            "teamId": "team-1",
            "fromMemberId": null,
            "toMemberId": "member-1",
            "text": "Review this",
            "timestamp": "2026-07-10T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(message.kind, TeamMailboxMessageKind::Message);
        assert!(!message.delivered);

        assert_eq!(
            serde_json::to_string(&TeamMailboxMessageKind::Message).unwrap(),
            "\"MESSAGE\""
        );
        assert_eq!(
            serde_json::to_string(&TeamMailboxMessageKind::NewTask).unwrap(),
            "\"NEW_TASK\""
        );
        assert_eq!(
            serde_json::to_string(&TeamMailboxMessageKind::FinalAnswer).unwrap(),
            "\"FINAL_ANSWER\""
        );
    }
}
