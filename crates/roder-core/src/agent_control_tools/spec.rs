use std::sync::Arc;

use roder_api::tools::{ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::json;

pub(super) const SPAWN_AGENT: &str = "spawn_agent";
pub(super) const SEND_MESSAGE: &str = "send_message";
pub(super) const FOLLOWUP_TASK: &str = "followup_task";
pub(super) const WAIT_AGENT: &str = "wait_agent";
pub(super) const LIST_AGENTS: &str = "list_agents";
pub(super) const INTERRUPT_AGENT: &str = "interrupt_agent";
pub(super) const MIN_WAIT_TIMEOUT_MS: u64 = 10_000;
pub(super) const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
pub(super) const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;

pub(crate) fn contribute_agent_control_tools(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    for kind in AgentControlToolKind::all() {
        registry.register(Arc::new(AgentControlTool { kind }))?;
    }
    Ok(())
}

pub(crate) fn is_agent_control_tool(name: &str) -> bool {
    matches!(
        name,
        SPAWN_AGENT | SEND_MESSAGE | FOLLOWUP_TASK | WAIT_AGENT | LIST_AGENTS | INTERRUPT_AGENT
    )
}

#[derive(Debug, Clone, Copy)]
enum AgentControlToolKind {
    SpawnAgent,
    SendMessage,
    FollowupTask,
    WaitAgent,
    ListAgents,
    InterruptAgent,
}

impl AgentControlToolKind {
    fn all() -> [Self; 6] {
        [
            Self::SpawnAgent,
            Self::SendMessage,
            Self::FollowupTask,
            Self::WaitAgent,
            Self::ListAgents,
            Self::InterruptAgent,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            Self::SpawnAgent => SPAWN_AGENT,
            Self::SendMessage => SEND_MESSAGE,
            Self::FollowupTask => FOLLOWUP_TASK,
            Self::WaitAgent => WAIT_AGENT,
            Self::ListAgents => LIST_AGENTS,
            Self::InterruptAgent => INTERRUPT_AGENT,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::SpawnAgent => {
                "Spawn a subagent for a concrete bounded task. The child gets a canonical task path, inherits the live model and reasoning by default, and can recursively use the same agent-control tools up to five levels below /root."
            }
            Self::SendMessage => {
                "Queue a message for an existing agent without starting a new turn. Running agents receive it at an inference boundary."
            }
            Self::FollowupTask => {
                "Assign follow-up work to an existing non-root agent and start a turn when it is idle."
            }
            Self::WaitAgent => "Wait for one or more agents to report a terminal result.",
            Self::ListAgents => "List live agents in the caller's canonical task tree.",
            Self::InterruptAgent => {
                "Interrupt an agent's current turn without closing its reusable identity."
            }
        }
    }

    fn parameters(self) -> serde_json::Value {
        match self {
            Self::SpawnAgent => json!({
                "type": "object",
                "properties": {
                    "task_name": {
                        "type": "string",
                        "description": "Stable lowercase task name for the spawned subagent."
                    },
                    "message": {
                        "type": "string",
                        "description": "Initial task or prompt for the spawned subagent."
                    },
                    "agent_type": {
                        "type": "string",
                        "description": "Optional collaboration label for the subagent (for example, release-audit). This label is allowed with a full-history fork and does not select a model, provider, or tool set."
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model override for the spawned subagent."
                    },
                    "model_provider": {
                        "type": "string",
                        "description": "Optional provider override for the spawned subagent."
                    },
                    "reasoning_effort": {
                        "type": "string",
                        "description": "Optional reasoning-effort override. Omit to inherit the parent effort."
                    },
                    "fork_turns": {
                        "type": "string",
                        "description": "Context to fork: `all` (default), `none`, or a positive integer string for the latest N turns. Full-history forks inherit the parent model, provider, and reasoning effort; agent_type remains an allowed collaboration label, but model, model_provider, and reasoning_effort overrides are rejected."
                    }
                },
                "required": ["task_name", "message"],
                "additionalProperties": false
            }),
            Self::SendMessage | Self::FollowupTask => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Agent target from spawn_agent or list_agents. Accepts a task name, member id, thread id, canonical path, or relative path."
                    },
                    "message": {
                        "type": "string",
                        "description": "Message text for the subagent."
                    }
                },
                "required": ["target", "message"],
                "additionalProperties": false
            }),
            Self::WaitAgent => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Optional subagent target. Omit to wait for any subagent owned by this caller thread."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": MIN_WAIT_TIMEOUT_MS,
                        "maximum": MAX_WAIT_TIMEOUT_MS,
                        "description": "Maximum time to wait in milliseconds. Defaults to 30000."
                    }
                },
                "additionalProperties": false
            }),
            Self::ListAgents => json!({
                "type": "object",
                "properties": {
                    "path_prefix": {
                        "type": "string",
                        "description": "Optional task-name prefix."
                    }
                },
                "additionalProperties": false
            }),
            Self::InterruptAgent => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Non-root agent target from spawn_agent or list_agents."
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        }
    }
}

struct AgentControlTool {
    kind: AgentControlToolKind,
}

#[async_trait::async_trait]
impl ToolExecutor for AgentControlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.kind.name().to_string(),
            description: self.kind.description().to_string(),
            parameters: self.kind.parameters(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: roder_api::tools::ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "agent control tools are executed by the runtime control plane".to_string(),
            data: json!({
                "error": {
                    "kind": "runtime_control_required",
                    "message": "agent control tools must run through Runtime::route_tool_call"
                }
            }),
            is_error: true,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SpawnAgentArgs {
    pub(super) task_name: String,
    pub(super) message: String,
    #[serde(default)]
    pub(super) agent_type: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) model_provider: Option<String>,
    #[serde(default)]
    pub(super) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(super) fork_turns: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MessageAgentArgs {
    pub(super) target: String,
    pub(super) message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WaitAgentArgs {
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListAgentsArgs {
    #[serde(default)]
    pub(super) path_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InterruptAgentArgs {
    pub(super) target: String,
}

pub(super) fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn normalize_fork_turns(value: Option<&str>) -> Result<String, &'static str> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    match value {
        None => Ok("all".to_string()),
        Some(value) if value.eq_ignore_ascii_case("all") => Ok("all".to_string()),
        Some(value) if value.eq_ignore_ascii_case("none") => Ok("none".to_string()),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|turns| *turns > 0)
            .map(|turns| turns.to_string())
            .ok_or("fork_turns must be `none`, `all`, or a positive integer string"),
    }
}

pub(super) fn valid_task_name(task_name: &str) -> bool {
    !task_name.is_empty()
        && task_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(super) fn full_history_selection_overrides_present(
    fork_turns: &str,
    model: Option<&str>,
    model_provider: Option<&str>,
    reasoning_effort: Option<&str>,
) -> bool {
    fork_turns == "all"
        && (model.is_some() || model_provider.is_some() || reasoning_effort.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_v2_tool_surface_uses_interrupt_without_close_alias() {
        let names = AgentControlToolKind::all()
            .into_iter()
            .map(AgentControlToolKind::name)
            .collect::<Vec<_>>();
        assert!(names.contains(&INTERRUPT_AGENT));
        assert!(!names.contains(&"close_agent"));
        assert!(!is_agent_control_tool("close_agent"));
    }

    #[test]
    fn fork_turns_defaults_to_all_and_normalizes_supported_values() {
        assert_eq!(normalize_fork_turns(None), Ok("all".to_string()));
        assert_eq!(normalize_fork_turns(Some(" ALL ")), Ok("all".to_string()));
        assert_eq!(normalize_fork_turns(Some("none")), Ok("none".to_string()));
        assert_eq!(normalize_fork_turns(Some("003")), Ok("3".to_string()));
        assert!(normalize_fork_turns(Some("0")).is_err());
        assert!(normalize_fork_turns(Some("banana")).is_err());
    }

    #[test]
    fn task_names_are_stable_canonical_path_segments() {
        for valid in ["worker", "task_3", "5x"] {
            assert!(valid_task_name(valid), "{valid}");
        }
        for invalid in ["", "Task", "two words", "parent/child", "task-3"] {
            assert!(!valid_task_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn full_history_forks_reject_selection_overrides_but_allow_labels() {
        assert!(full_history_selection_overrides_present(
            "all",
            Some("gpt-5.6-terra"),
            None,
            None
        ));
        assert!(full_history_selection_overrides_present(
            "all",
            None,
            Some("codex"),
            None
        ));
        assert!(full_history_selection_overrides_present(
            "all",
            None,
            None,
            Some("high")
        ));
        assert!(!full_history_selection_overrides_present(
            "3",
            Some("gpt-5.6-terra"),
            Some("codex"),
            Some("high")
        ));
        assert!(!full_history_selection_overrides_present(
            "all", None, None, None
        ));
    }

    #[test]
    fn spawn_schema_allows_a_label_with_full_history() {
        let schema = AgentControlToolKind::SpawnAgent.parameters();
        let agent_type = schema
            .pointer("/properties/agent_type/description")
            .and_then(serde_json::Value::as_str)
            .expect("agent_type description");
        let fork_turns = schema
            .pointer("/properties/fork_turns/description")
            .and_then(serde_json::Value::as_str)
            .expect("fork_turns description");

        assert!(agent_type.contains("allowed with a full-history fork"));
        assert!(fork_turns.contains("agent_type remains an allowed collaboration label"));
    }

    #[test]
    fn wait_schema_advertises_codex_v2_timeout_bounds() {
        let schema = AgentControlToolKind::WaitAgent.parameters();
        assert_eq!(
            schema.pointer("/properties/timeout_ms/minimum"),
            Some(&json!(MIN_WAIT_TIMEOUT_MS))
        );
        assert_eq!(
            schema.pointer("/properties/timeout_ms/maximum"),
            Some(&json!(MAX_WAIT_TIMEOUT_MS))
        );
    }
}
