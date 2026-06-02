use std::sync::Arc;

use roder_api::context::{PolicyContribution, PolicyContributor, PolicyGate, PolicyReview};
use roder_api::policy_mode::{PolicyDecision, PolicyMode, PolicyModeConfig};
use roder_api::tools::{ToolCall, ToolExecutionContext};

#[derive(Debug, Clone, Default)]
pub struct DefaultPolicyGate;

impl DefaultPolicyGate {
    pub fn new() -> Self {
        Self
    }

    pub async fn decide_with_contributors(
        &self,
        call: &ToolCall,
        mode: PolicyMode,
        context: &ToolExecutionContext,
        contributors: &[Arc<dyn PolicyContributor>],
    ) -> anyhow::Result<PolicyDecision> {
        let mut decision = self.decide(call, mode, context);
        for contributor in contributors {
            let contribution = contributor
                .review_tool(PolicyReview {
                    call: call.clone(),
                    mode,
                    context: context.clone(),
                })
                .await?;
            decision = merge_policy_decision(decision, contributor.id(), contribution);
        }
        Ok(decision)
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

fn merge_policy_decision(
    current: PolicyDecision,
    contributor_id: String,
    contribution: PolicyContribution,
) -> PolicyDecision {
    match (current, contribution) {
        (PolicyDecision::Denied { reason }, _) => PolicyDecision::Denied { reason },
        (_, PolicyContribution::Deny { reason }) => PolicyDecision::Denied {
            reason: format!("policy contributor {contributor_id} denied tool call: {reason}"),
        },
        (PolicyDecision::RequiresApproval { reason }, _) => {
            PolicyDecision::RequiresApproval { reason }
        }
        (_, PolicyContribution::RequireApproval { reason }) => {
            PolicyDecision::RequiresApproval { reason }
        }
        (decision @ PolicyDecision::AutoApproved { .. }, PolicyContribution::Abstain) => decision,
        (decision @ PolicyDecision::AutoApproved { .. }, PolicyContribution::Allow { .. }) => {
            decision
        }
        (
            PolicyDecision::Allowed,
            PolicyContribution::Abstain | PolicyContribution::Allow { .. },
        ) => PolicyDecision::Allowed,
    }
}

fn looks_like_side_effect(call: &ToolCall) -> bool {
    looks_like_write(call) || looks_like_process(call)
}

fn looks_like_write(call: &ToolCall) -> bool {
    if matches!(
        call.name.as_str(),
        "roadmap_create"
            | "roadmap_set_task_state"
            | "roadmap_thread_attach"
            | "vcs/select"
            | "vcs/snapshot/create"
            | "vcs/restore"
            | "vcs/lines/switch"
    ) {
        return true;
    }
    tool_name_contains_any(
        call,
        &[
            "write", "edit", "patch", "delete", "mkdir", "move", "rename",
        ],
    )
}

fn looks_like_process(call: &ToolCall) -> bool {
    if matches!(call.name.as_str(), "vcs/sync") {
        return true;
    }
    tool_name_contains_any(
        call,
        &[
            "process", "spawn", "shell", "bash", "exec", "terminal", "command",
        ],
    )
}

fn looks_like_network(call: &ToolCall) -> bool {
    tool_name_contains_any(
        call,
        &["network", "web_search", "fetch", "download", "http", "url"],
    )
}

fn tool_name_contains_any(call: &ToolCall, signals: &[&str]) -> bool {
    let name = call.name.to_ascii_lowercase();
    signals.iter().any(|signal| name.contains(signal))
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::tools::ToolExecutionContext;
    use serde_json::json;

    use super::*;

    #[test]
    fn plan_mode_allows_read_like_tool_with_write_like_arguments() {
        let decision = DefaultPolicyGate::new().decide(
            &call(
                "read_metadata",
                json!({ "operation": "fs.write", "path": "src/lib.rs" }),
            ),
            PolicyMode::Plan,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::Allowed));
    }

    #[test]
    fn grep_query_containing_destructive_words_is_allowed() {
        let decision = DefaultPolicyGate::new().decide(
            &call(
                "grep",
                json!({ "query": "edit command patch", "path": "." }),
            ),
            PolicyMode::Default,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::Allowed));
    }

    #[test]
    fn plan_mode_denies_write_tool_name() {
        let decision = DefaultPolicyGate::new().decide(
            &call("fs.write", json!({ "path": "src/lib.rs" })),
            PolicyMode::Plan,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::Denied { .. }));
    }

    #[test]
    fn plan_mode_denies_shell_tool_name() {
        let decision = DefaultPolicyGate::new().decide(
            &call("shell", json!({ "command": "cargo test" })),
            PolicyMode::Plan,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::Denied { .. }));
    }

    #[test]
    fn default_mode_shell_still_requires_approval() {
        let decision = DefaultPolicyGate::new().decide(
            &call("shell", json!({ "command": "cargo test" })),
            PolicyMode::Default,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
    }

    #[test]
    fn default_mode_edit_still_requires_approval() {
        let decision = DefaultPolicyGate::new().decide(
            &call("fs.edit", json!({ "path": "src/lib.rs" })),
            PolicyMode::Default,
            &context(),
        );

        assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
    }

    #[test]
    fn roadmap_mutating_tools_follow_write_policy() {
        for tool in [
            "roadmap_create",
            "roadmap_patch",
            "roadmap_set_task_state",
            "roadmap_thread_attach",
        ] {
            let default_decision = DefaultPolicyGate::new().decide(
                &call(tool, json!({})),
                PolicyMode::Default,
                &context(),
            );
            assert!(
                matches!(default_decision, PolicyDecision::RequiresApproval { .. }),
                "{tool} should require approval in default mode"
            );

            let plan_decision = DefaultPolicyGate::new().decide(
                &call(tool, json!({})),
                PolicyMode::Plan,
                &context(),
            );
            assert!(
                matches!(plan_decision, PolicyDecision::Denied { .. }),
                "{tool} should be denied in plan mode"
            );
        }
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
        ToolExecutionContext::new(
            ThreadId::from("thread-1"),
            TurnId::from("turn-1"),
            PolicyMode::Default,
        )
    }
}
