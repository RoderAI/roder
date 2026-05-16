use roder_api::context::PolicyGate;
use roder_api::policy_mode::{PolicyDecision, PolicyMode, PolicyModeConfig};
use roder_api::tools::{ToolCall, ToolExecutionContext};

#[derive(Debug, Clone, Default)]
pub struct DefaultPolicyGate;

impl DefaultPolicyGate {
    pub fn new() -> Self {
        Self
    }
}

impl PolicyGate for DefaultPolicyGate {
    fn decide(
        &self,
        call: &ToolCall,
        mode: PolicyMode,
        _context: &ToolExecutionContext,
    ) -> PolicyDecision {
        let config = PolicyModeConfig::for_mode(mode);
        if config.denied_tools.iter().any(|tool| tool == &call.name) {
            return PolicyDecision::Denied {
                reason: format!("tool {:?} is denied by policy", call.name),
            };
        }
        if !config.allow_writes && looks_like_write(call) {
            return PolicyDecision::Denied {
                reason: "write-like tool calls are denied in the active policy mode".to_string(),
            };
        }
        if !config.allow_process && looks_like_process(call) {
            return PolicyDecision::Denied {
                reason: "process-like tool calls are denied in the active policy mode".to_string(),
            };
        }
        if !config.allow_network && looks_like_network(call) {
            return PolicyDecision::Denied {
                reason: "network-like tool calls are denied in the active policy mode".to_string(),
            };
        }
        if config.auto_approve.contains_tool(&call.name) {
            return PolicyDecision::AutoApproved {
                matched_rule: matching_rule(&config, &call.name),
            };
        }
        if looks_like_side_effect(call) {
            return PolicyDecision::RequiresApproval {
                reason: Some("side-effecting tool call".to_string()),
            };
        }
        PolicyDecision::Allowed
    }
}

fn matching_rule(config: &PolicyModeConfig, tool_name: &str) -> Option<String> {
    config
        .auto_approve
        .tools
        .iter()
        .find(|tool| tool.as_str() == "*" || tool.as_str() == tool_name)
        .cloned()
}

fn looks_like_side_effect(call: &ToolCall) -> bool {
    looks_like_write(call) || looks_like_process(call) || looks_like_network(call)
}

fn looks_like_write(call: &ToolCall) -> bool {
    contains_any_signal(
        call,
        &[
            "fs.write",
            "fs.edit",
            "fs.multi_edit",
            "write",
            "edit",
            "patch",
            "delete",
            "mkdir",
            "move",
            "rename",
        ],
    )
}

fn looks_like_process(call: &ToolCall) -> bool {
    contains_any_signal(
        call,
        &[
            "process.spawn",
            "process",
            "spawn",
            "shell",
            "bash",
            "exec",
            "terminal",
            "command",
        ],
    )
}

fn looks_like_network(call: &ToolCall) -> bool {
    contains_any_signal(
        call,
        &["network", "web_search", "fetch", "download", "http", "url"],
    )
}

fn contains_any_signal(call: &ToolCall, signals: &[&str]) -> bool {
    let name = call.name.to_ascii_lowercase();
    if signals.iter().any(|signal| name.contains(signal)) {
        return true;
    }
    let arguments = call.arguments.to_string().to_ascii_lowercase();
    signals.iter().any(|signal| arguments.contains(signal))
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::tools::ToolExecutionContext;
    use serde_json::json;

    use super::*;

    #[test]
    fn plan_mode_denies_write_like_arguments_even_with_benign_tool_name() {
        let decision = DefaultPolicyGate::new().decide(
            &call(
                "read_metadata",
                json!({ "operation": "fs.write", "path": "src/lib.rs" }),
            ),
            PolicyMode::Plan,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::Denied { .. }));
    }

    #[test]
    fn accept_all_auto_approves_process_spawn() {
        let decision = DefaultPolicyGate::new().decide(
            &call("process.spawn", json!({ "cmd": "cargo test" })),
            PolicyMode::AcceptAll,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::AutoApproved { .. }));
    }

    #[test]
    fn accept_all_auto_approves_shell_tool() {
        let decision = DefaultPolicyGate::new().decide(
            &call("shell", json!({ "command": "cargo test" })),
            PolicyMode::AcceptAll,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::AutoApproved { .. }));
    }

    #[test]
    fn bypass_auto_approves_tools_without_overriding_denies() {
        let decision = DefaultPolicyGate::new().decide(
            &call("process.spawn", json!({ "cmd": "cargo test" })),
            PolicyMode::Bypass,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::AutoApproved { .. }));
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call-1".to_string(),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        }
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext {
            thread_id: ThreadId::from("thread-1"),
            turn_id: TurnId::from("turn-1"),
            effective_mode: PolicyMode::Default,
        }
    }
}
