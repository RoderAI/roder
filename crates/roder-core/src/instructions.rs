use roder_api::inference::{
    InstructionBundle, ModelHarnessProfile, ModelInstructionOverlay, RuntimeProfile,
};

pub const RODER_INSTRUCTIONS: &str = r#"You are Roder, a Rust-native coding agent running inside a terminal TUI on a user's computer.

Roder is inspired by OpenAI Codex and the original Gode agent harness. Within this context, "Roder" refers to this open-source coding-agent harness and TUI, not a language model.

## How You Work

- Be precise, safe, and helpful.
- Keep responses concise and direct unless the user asks for detail.
- Prefer actionable guidance and concrete next steps.
- Continue working until the user's coding task is genuinely handled.
- Use the tools provided by the harness to inspect files, search the workspace, and make progress.

## Workspace And Tools

- Treat the current workspace as the user's repository.
- When searching for text or files, prefer fast targeted search. If a search tool is available, use it before broad manual inspection.
- Read relevant files before making assumptions about the codebase.
- Keep edits scoped to the user's request and consistent with existing project patterns.
- The available tool set depends on how this Roder thread is configured. Do not claim access to tools that are not exposed in the current turn.
- Roder exposes tools through its Responses namespace and tool search. Use the available tool names directly; do not assume a legacy functions namespace or that only the most recently used tool is available.
- When discovery tools are available, use `discovery.list`, `discovery.search`, or `discovery.read` before using unfamiliar tools, MCP servers, skills, commands, plugins, subagents, or file-backed artifact surfaces. Reading a discovery item promotes its detailed schema or instructions for the thread.

## Editing Constraints

- Default to ASCII when editing or creating files. Only introduce non-ASCII when clearly justified or when the file already uses it.
- Add succinct comments only when they clarify non-obvious logic.
- Prefer dedicated file-editing tools (`apply_patch`, `edit`, `multi_edit`, or `write_file`) for source changes when they are available. Use shell commands for editing only when the requested edit cannot be expressed with the available editing tools or those tools fail.
- Do not revert changes you did not make unless the user explicitly asks.
- You may be in a dirty git worktree. Ignore unrelated work from other agents or the user.
- Do not use destructive operations such as hard resets or deleting user work unless explicitly requested.

## Validation

- When you change code, run the most relevant tests or build commands available for the touched area.
- Start with focused checks, then broaden when confidence increases.
- If you cannot run a useful validation command, say exactly what was not verified and why.

## Communication

- Before making tool calls, send a brief preamble message to the user explaining what you are about to do.
- Group related tool calls under one concise preamble instead of narrating every trivial read separately.
- Keep preambles to 1-2 sentences focused on immediate, tangible next steps; for quick updates, aim for 8-12 words.
- Build on prior context in later preambles so the user can follow progress and understand the next action.
- Explain what changed and why in plain engineering language.
- If an operation fails, surface the key error and the likely next debugging step.
- Avoid dumping large files or logs into the response; summarize and reference paths where useful."#;

pub fn default_instructions() -> InstructionBundle {
    let mut system = RODER_INSTRUCTIONS.to_string();
    if cfg!(target_os = "windows") {
        system.push_str(WINDOWS_SYSTEM_INSTRUCTIONS);
    }
    InstructionBundle {
        system: Some(system),
        developer: None,
        developer_context: None,
    }
}

const WINDOWS_SYSTEM_INSTRUCTIONS: &str = r#"

## Windows Runtime

- You are running on Windows.
- Prefer PowerShell commands and PowerShell syntax for shell operations. This shell guidance does not supersede the instruction to prefer dedicated file-editing tools for source changes.
- Use Windows paths and commands when referring to files, processes, environment variables, and filesystem operations."#;

const NON_INTERACTIVE_INSTRUCTIONS: &str = r#"## Runtime Profile

This turn is running in a non-interactive profile. Do not wait for unavailable user clarification. Assume reasonable defaults, state assumptions briefly when needed, and continue to a concrete final result."#;

const EVAL_INSTRUCTIONS: &str = r#"## Eval Runtime Profile

This turn is running in eval mode. Do not wait for user clarification unless explicit fixture answers are available. Assume reasonable defaults, keep progress observable through tools and events, and reach a final answer only after the task has been handled."#;

const TASK_LEDGER_REQUIRED_INSTRUCTIONS: &str = r#"## Task Ledger Required

This eval task is decomposed work. The first tool call must be `task_ledger.update`; do not call shell, search, web, file, or edit tools before the ledger exists. Keep exactly one item in progress and include evidence when marking items completed."#;

const PLAN_MODE_INSTRUCTIONS: &str = r#"## Plan Mode

You are in plan mode. Do not make file changes or run implementation commands yet. Inspect and discuss as needed, then present a concrete implementation plan to the user.

When the plan is ready for approval, call `exit_plan_mode` with:
- `summary`: the user-visible plan, in Markdown.
- `next_steps`: concise implementation steps.
- `target_mode`: `default` unless the user explicitly asked for a more permissive mode.

After the user approves, the harness exits plan mode and the turn may continue with implementation. If the user rejects, keep discussing and revise the plan."#;

const LITERAL_TOOL_OUTPUTS_OVERLAY: &str = r#"## Model Harness Profile

Tool outputs are literal evidence from the harness. Prefer exact filenames, command output, and structured tool results over inferred state."#;

const INTUITIVE_CONTEXT_OVERLAY: &str = r#"## Model Harness Profile

Use the provided context as the current working set. Ask for or inspect missing files before assuming project structure outside the visible evidence."#;

/// Prepends host-supplied thread instructions to the developer slot so they layer directly under
/// the harness system prompt while harness addenda (runtime profile, plan mode, overlays) append
/// after them.
pub fn apply_thread_developer_instructions(
    mut instructions: InstructionBundle,
    addition: &str,
) -> InstructionBundle {
    let addition = addition.trim();
    if addition.is_empty() {
        return instructions;
    }
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => format!("{addition}\n\n{existing}"),
        _ => addition.to_string(),
    });
    instructions
}

/**
 * Sets the per-turn developer-context slot. Supplied on turn/start only and
 * never persisted, so a context delivered on turn N is absent on turn N+1
 * unless the host sends it again. Providers render the slot after all stable
 * instruction content — through a provider-native per-turn channel (e.g. a
 * trailing system-role message) where available — so cached stable-prefix
 * blocks survive per-turn changes.
 */
pub fn apply_turn_developer_context(
    mut instructions: InstructionBundle,
    context: &str,
) -> InstructionBundle {
    let context = context.trim();
    if context.is_empty() {
        return instructions;
    }
    instructions.developer_context = Some(context.to_string());
    instructions
}

pub fn apply_runtime_profile(
    mut instructions: InstructionBundle,
    profile: RuntimeProfile,
) -> InstructionBundle {
    let addition = match profile {
        RuntimeProfile::Interactive => return instructions,
        RuntimeProfile::NonInteractive => NON_INTERACTIVE_INSTRUCTIONS,
        RuntimeProfile::Eval => EVAL_INSTRUCTIONS,
    };
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{addition}"),
        _ => addition.to_string(),
    });
    instructions
}

pub fn apply_task_ledger_required(mut instructions: InstructionBundle) -> InstructionBundle {
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{TASK_LEDGER_REQUIRED_INSTRUCTIONS}")
        }
        _ => TASK_LEDGER_REQUIRED_INSTRUCTIONS.to_string(),
    });
    instructions
}

pub fn apply_plan_mode(mut instructions: InstructionBundle) -> InstructionBundle {
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{PLAN_MODE_INSTRUCTIONS}")
        }
        _ => PLAN_MODE_INSTRUCTIONS.to_string(),
    });
    instructions
}

/// Inject the agent-swarm reminder into the developer instructions while
/// agent-swarm mode is active, so any app-server/SDK client (not just the TUI)
/// nudges the model toward the `agent_swarm` fanout tool.
pub fn apply_agent_swarm_mode(mut instructions: InstructionBundle) -> InstructionBundle {
    let reminder = roder_api::subagents::AGENT_SWARM_MODE_REMINDER;
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{reminder}"),
        _ => reminder.to_string(),
    });
    instructions
}

pub fn apply_model_instruction_overlay(
    mut instructions: InstructionBundle,
    profile: &ModelHarnessProfile,
) -> InstructionBundle {
    let addition = match profile.instruction_overlay {
        ModelInstructionOverlay::Standard => return instructions,
        ModelInstructionOverlay::LiteralToolOutputs => LITERAL_TOOL_OUTPUTS_OVERLAY,
        ModelInstructionOverlay::IntuitiveContext => INTUITIVE_CONTEXT_OVERLAY,
    };
    instructions.developer = Some(match instructions.developer {
        Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{addition}"),
        _ => addition.to_string(),
    });
    instructions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_swarm_mode_injects_reminder_into_developer_instructions() {
        let injected = apply_agent_swarm_mode(InstructionBundle::default());
        let developer = injected.developer.expect("developer instructions");
        assert!(developer.contains("agent_swarm"));
        assert!(developer.contains("{{item}}"));

        // Appends to existing developer instructions rather than replacing them.
        let with_existing = apply_agent_swarm_mode(InstructionBundle {
            developer: Some("existing dev rules".to_string()),
            ..InstructionBundle::default()
        });
        let developer = with_existing.developer.expect("developer instructions");
        assert!(developer.starts_with("existing dev rules"));
        assert!(developer.contains("agent_swarm"));
    }

    #[test]
    fn base_instructions_name_lazy_discovery_tools() {
        let instructions = default_instructions();
        let system = instructions.system.expect("system instructions");
        assert!(system.contains("discovery.list"));
        assert!(system.contains("discovery.search"));
        assert!(system.contains("discovery.read"));
        assert!(system.contains("promotes its detailed schema"));
    }

    #[test]
    fn windows_instructions_match_host_platform() {
        let instructions = default_instructions();
        let system = instructions.system.expect("system instructions");
        assert_eq!(
            system.contains("You are running on Windows."),
            cfg!(target_os = "windows")
        );
        assert_eq!(
            system.contains("Prefer PowerShell commands"),
            cfg!(target_os = "windows")
        );
        assert_eq!(
            system.contains(
                "does not supersede the instruction to prefer dedicated file-editing tools"
            ),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn base_instructions_prefer_edit_tools_for_source_changes() {
        let instructions = default_instructions();
        let system = instructions.system.expect("system instructions");
        assert!(system.contains("Prefer dedicated file-editing tools"));
        assert!(system.contains("apply_patch"));
    }

    #[test]
    fn plan_mode_instructions_tell_model_to_request_approval() {
        let instructions = apply_plan_mode(InstructionBundle::default());
        let developer = instructions.developer.expect("developer instructions");
        assert!(developer.contains("You are in plan mode"));
        assert!(developer.contains("exit_plan_mode"));
        assert!(developer.contains("After the user approves"));
    }

    #[test]
    fn thread_developer_instructions_prepend_to_developer_slot() {
        let instructions = apply_runtime_profile(
            apply_thread_developer_instructions(
                default_instructions(),
                "You are embedded in a host app.",
            ),
            RuntimeProfile::NonInteractive,
        );
        let developer = instructions.developer.expect("developer instructions");
        assert!(developer.starts_with("You are embedded in a host app."));
        assert!(developer.contains("non-interactive profile"));
        assert!(
            instructions
                .system
                .expect("system")
                .starts_with("You are Roder")
        );

        let unchanged = apply_thread_developer_instructions(default_instructions(), "   ");
        assert_eq!(unchanged.developer, None);
    }

    #[test]
    fn turn_developer_context_fills_dedicated_slot_after_thread_instructions() {
        let instructions = apply_turn_developer_context(
            apply_runtime_profile(
                apply_thread_developer_instructions(
                    default_instructions(),
                    "You are embedded in a host app.",
                ),
                RuntimeProfile::NonInteractive,
            ),
            "Connected accounts: example-service.",
        );
        let developer = instructions.developer.expect("developer instructions");
        assert!(developer.starts_with("You are embedded in a host app."));
        assert!(!developer.contains("Connected accounts"));
        assert_eq!(
            instructions.developer_context.as_deref(),
            Some("Connected accounts: example-service.")
        );

        let unchanged = apply_turn_developer_context(default_instructions(), "   ");
        assert_eq!(unchanged.developer_context, None);
    }

    #[test]
    fn base_instructions_include_intermediary_message_guidance() {
        let instructions = default_instructions();
        let system = instructions.system.expect("system instructions");
        assert!(system.starts_with("You are Roder"));
        assert!(system.contains("Before making tool calls, send a brief preamble message"));
        assert!(system.contains("Group related tool calls under one concise preamble"));
        assert!(system.contains("Build on prior context in later preambles"));
    }
}
