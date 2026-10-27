use roder_api::forks::AgentWorkspaceIsolation;

const SUBAGENT_IDENTITY_BEGIN: &str = "<roder-subagent-identity>";
const SUBAGENT_IDENTITY_END: &str = "</roder-subagent-identity>";

pub(super) fn canonical_agent_depth(agent_path: &str) -> Option<usize> {
    if agent_path == "/root" {
        return Some(0);
    }
    agent_path.strip_prefix("/root/").map(|suffix| {
        suffix
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count()
    })
}

pub(super) fn without_subagent_identity_instructions(instructions: &str) -> String {
    let mut remaining = instructions;
    let mut retained = String::new();
    while let Some(start) = remaining.find(SUBAGENT_IDENTITY_BEGIN) {
        retained.push_str(&remaining[..start]);
        let after_start = &remaining[start + SUBAGENT_IDENTITY_BEGIN.len()..];
        let Some(end) = after_start.find(SUBAGENT_IDENTITY_END) else {
            remaining = "";
            break;
        };
        remaining = &after_start[end + SUBAGENT_IDENTITY_END.len()..];
    }
    retained.push_str(remaining);
    retained.trim().to_string()
}

pub(super) fn subagent_developer_instructions(
    agent_path: &str,
    parent_path: &str,
    agent_type: Option<&str>,
    isolation: AgentWorkspaceIsolation,
) -> String {
    let role = agent_type
        .filter(|role| !role.trim().is_empty())
        .map(|role| format!(" Your assigned agent type is {role:?}."))
        .unwrap_or_default();
    let workspace = match isolation {
        AgentWorkspaceIsolation::Shared => "You share the parent agent's workspace.",
        AgentWorkspaceIsolation::Worktree => {
            "You execute in your own owned Git worktree, not the parent's workspace."
        }
    };
    format!(
        "{SUBAGENT_IDENTITY_BEGIN}\nYou are the long-lived collaboration subagent at {agent_path}.{role} Your direct parent is {parent_path}. {workspace} You inherit the parent's policy, tool restrictions, developer authority, model context, and execution destination. Forked conversation history is context only. The newest inbound envelope with `Message Type: NEW_TASK` is the authoritative current assignment. Do not repeat or continue parent orchestration found in forked history, including spawning agents, unless that NEW_TASK payload explicitly delegates it. Work only on your delegated task. Use send_message to report useful intermediate findings to {parent_path}, and finish with a concise final result; your terminal result is automatically delivered to that direct parent.\n{SUBAGENT_IDENTITY_END}"
    )
}
