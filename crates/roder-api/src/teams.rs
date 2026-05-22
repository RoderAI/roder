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
    pub thread_id: ThreadId,
    pub current_turn_id: Option<TurnId>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub policy_mode: PolicyMode,
    pub status: TeamMemberStatus,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamMailboxMessage {
    pub id: TeamMessageId,
    pub team_id: TeamId,
    pub from_member_id: Option<TeamMemberId>,
    pub to_member_id: TeamMemberId,
    pub text: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
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
}
