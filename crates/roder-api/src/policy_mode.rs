use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    #[default]
    Default,
    #[serde(alias = "accept_edits", alias = "accept-edits")]
    AcceptAll,
    Plan,
    Bypass,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct AutoApproveSet {
    pub tools: Vec<String>,
}

impl AutoApproveSet {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn all() -> Self {
        Self {
            tools: vec!["*".to_string()],
        }
    }

    pub fn accept_all() -> Self {
        Self {
            tools: vec![
                "fs.write".to_string(),
                "fs.edit".to_string(),
                "fs.multi_edit".to_string(),
                "apply_patch".to_string(),
                "write_file".to_string(),
                "edit".to_string(),
                "multi_edit".to_string(),
                "process.spawn".to_string(),
                "shell".to_string(),
                "exec_command".to_string(),
                "write_stdin".to_string(),
                "vcs/select".to_string(),
                "vcs/snapshot/create".to_string(),
                "vcs/restore".to_string(),
                "vcs/lines/switch".to_string(),
                "vcs/sync".to_string(),
            ],
        }
    }

    pub fn contains_tool(&self, tool_name: &str) -> bool {
        self.tools
            .iter()
            .any(|tool| tool == "*" || tool == tool_name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyModeConfig {
    pub auto_approve: AutoApproveSet,
    pub denied_tools: Vec<String>,
    pub allow_writes: bool,
    pub allow_process: bool,
    pub allow_network: bool,
    pub requires_user_to_exit: bool,
}

impl PolicyModeConfig {
    pub fn for_mode(mode: PolicyMode) -> Self {
        match mode {
            PolicyMode::Default => Self::default_mode(),
            PolicyMode::AcceptAll => Self::accept_all(),
            PolicyMode::Plan => Self::plan(),
            PolicyMode::Bypass => Self::bypass(),
        }
    }

    pub fn default_mode() -> Self {
        Self {
            auto_approve: AutoApproveSet::empty(),
            denied_tools: Vec::new(),
            allow_writes: true,
            allow_process: true,
            allow_network: true,
            requires_user_to_exit: false,
        }
    }

    pub fn accept_all() -> Self {
        Self {
            auto_approve: AutoApproveSet::accept_all(),
            denied_tools: Vec::new(),
            allow_writes: true,
            allow_process: true,
            allow_network: true,
            requires_user_to_exit: false,
        }
    }

    pub fn plan() -> Self {
        Self {
            auto_approve: AutoApproveSet::empty(),
            denied_tools: Vec::new(),
            allow_writes: false,
            allow_process: false,
            allow_network: true,
            requires_user_to_exit: false,
        }
    }

    pub fn bypass() -> Self {
        Self {
            auto_approve: AutoApproveSet::all(),
            denied_tools: Vec::new(),
            allow_writes: true,
            allow_process: true,
            allow_network: true,
            requires_user_to_exit: false,
        }
    }
}

impl Default for PolicyModeConfig {
    fn default() -> Self {
        Self::default_mode()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision", content = "details")]
pub enum PolicyDecision {
    Allowed,
    RequiresApproval { reason: Option<String> },
    AutoApproved { matched_rule: Option<String> },
    Denied { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecisionRecorded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    pub mode: PolicyMode,
    pub decision: PolicyDecision,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBypassActive {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyModeChanged {
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub previous_mode: PolicyMode,
    pub new_mode: PolicyMode,
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExitPlanRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub target_mode: PolicyMode,
    pub plan_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_steps: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExitPlanResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub approved: bool,
    pub target_mode: PolicyMode,
    pub resolved_mode: PolicyMode,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_mode_serde_round_trips_as_snake_case() {
        let serialized = serde_json::to_string(&PolicyMode::AcceptAll).unwrap();
        assert_eq!(serialized, "\"accept_all\"");

        let round_trip: PolicyMode = serde_json::from_str(&serialized).unwrap();
        assert_eq!(round_trip, PolicyMode::AcceptAll);

        let legacy: PolicyMode = serde_json::from_str("\"accept_edits\"").unwrap();
        assert_eq!(legacy, PolicyMode::AcceptAll);
    }

    #[test]
    fn policy_mode_config_serde_round_trips() {
        let config = PolicyModeConfig {
            auto_approve: AutoApproveSet {
                tools: vec!["fs.write".to_string(), "network".to_string()],
            },
            denied_tools: vec!["process.spawn".to_string()],
            allow_writes: true,
            allow_process: false,
            allow_network: true,
            requires_user_to_exit: false,
        };

        let serialized = serde_json::to_string(&config).unwrap();
        let round_trip: PolicyModeConfig = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip, config);
        assert!(round_trip.auto_approve.contains_tool("fs.write"));
        assert!(!round_trip.auto_approve.contains_tool("fs.read"));
    }

    #[test]
    fn policy_decision_serde_round_trips() {
        let decision = PolicyDecision::RequiresApproval {
            reason: Some("write access".to_string()),
        };

        let serialized = serde_json::to_string(&decision).unwrap();
        let round_trip: PolicyDecision = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip, decision);
    }

    #[test]
    fn policy_event_payloads_round_trip() {
        let event = PolicyExitPlanRequested {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            request_id: "exit-1".to_string(),
            target_mode: PolicyMode::Default,
            plan_summary: Some("Implement the approved edits.".to_string()),
            next_steps: vec!["edit files".to_string(), "run tests".to_string()],
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };

        let serialized = serde_json::to_string(&event).unwrap();
        let round_trip: PolicyExitPlanRequested = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.request_id, "exit-1");
        assert_eq!(round_trip.target_mode, PolicyMode::Default);
        assert_eq!(
            round_trip.plan_summary.as_deref(),
            Some("Implement the approved edits.")
        );
        assert_eq!(round_trip.next_steps, ["edit files", "run tests"]);
    }

    #[test]
    fn policy_mode_default_presets_match_contract() {
        let default = PolicyModeConfig::for_mode(PolicyMode::Default);
        assert_eq!(default.auto_approve, AutoApproveSet::empty());
        assert!(default.allow_writes);
        assert!(default.allow_process);
        assert!(default.allow_network);
        assert!(!default.requires_user_to_exit);

        let accept_all = PolicyModeConfig::for_mode(PolicyMode::AcceptAll);
        assert!(accept_all.auto_approve.contains_tool("fs.write"));
        assert!(accept_all.auto_approve.contains_tool("fs.edit"));
        assert!(accept_all.auto_approve.contains_tool("fs.multi_edit"));
        assert!(accept_all.auto_approve.contains_tool("apply_patch"));
        assert!(accept_all.auto_approve.contains_tool("write_file"));
        assert!(accept_all.auto_approve.contains_tool("edit"));
        assert!(accept_all.auto_approve.contains_tool("multi_edit"));
        assert!(accept_all.auto_approve.contains_tool("process.spawn"));
        assert!(accept_all.auto_approve.contains_tool("shell"));
        assert!(accept_all.auto_approve.contains_tool("exec_command"));
        assert!(accept_all.auto_approve.contains_tool("write_stdin"));
        assert!(accept_all.allow_writes);
        assert!(accept_all.allow_process);
        assert!(accept_all.allow_network);

        let plan = PolicyModeConfig::for_mode(PolicyMode::Plan);
        assert_eq!(plan.auto_approve, AutoApproveSet::empty());
        assert!(!plan.allow_writes);
        assert!(!plan.allow_process);
        assert!(plan.allow_network);
        assert!(!plan.requires_user_to_exit);

        let bypass = PolicyModeConfig::for_mode(PolicyMode::Bypass);
        assert!(bypass.auto_approve.contains_tool("any.tool"));
        assert!(bypass.allow_writes);
        assert!(bypass.allow_process);
        assert!(bypass.allow_network);
        assert!(!bypass.requires_user_to_exit);
    }
}
