use roder_api::tools::ToolExecutionContext;

pub(crate) fn shell_for_context(ctx: &ToolExecutionContext, fallback: &str) -> String {
    ctx.command_shell
        .as_deref()
        .and_then(roder_api::command_shell::normalize_command_shell)
        .unwrap_or_else(|| fallback.to_string())
}
