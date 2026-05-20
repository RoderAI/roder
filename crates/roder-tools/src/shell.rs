use std::sync::Arc;
use std::time::Instant;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    registry.register(Arc::new(ShellTool { workspace }))
}

#[derive(Debug)]
struct ShellTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command in the workspace and return aggregated output."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command string evaluated by the user's configured shell."
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Optional working directory. Relative paths resolve from the workspace root. Defaults to the workspace root."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "default": DEFAULT_TIMEOUT_SECONDS
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_process_runner()?;
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
        let args = parse::<ShellArgs>(&call)?;
        let command = args.command.trim().to_string();
        require_nonempty(&command, "command")?;

        let cwd = match args.workdir.as_deref() {
            Some(workdir) if !workdir.trim().is_empty() => workspace.resolve_existing(workdir)?,
            _ => workspace.root().to_path_buf(),
        };
        let timeout = args
            .timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, 600);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let started = Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            Command::new(shell)
                .arg("-lc")
                .arg(&command)
                .current_dir(&cwd)
                .output(),
        )
        .await;

        let (exit_code, aggregated_output, timed_out) = match output {
            Ok(Ok(output)) => (
                output
                    .status
                    .code()
                    .unwrap_or_else(|| if output.status.success() { 0 } else { -1 }),
                aggregate_output(&output.stdout, &output.stderr),
                false,
            ),
            Ok(Err(err)) => (-1, format!("execution error: {err:?}"), false),
            Err(_) => (
                -1,
                format!("command timed out after {timeout} seconds"),
                true,
            ),
        };
        let duration_ms = started.elapsed().as_millis() as u64;
        let status = if exit_code == 0 && !timed_out {
            "completed"
        } else {
            "failed"
        };
        let text = format_shell_output(exit_code, duration_ms, &aggregated_output);

        Ok(result(
            call,
            text,
            json!({
                "command": command,
                "cwd": self.workspace.display(&cwd),
                "aggregated_output": aggregated_output,
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "status": status,
                "timed_out": timed_out,
            }),
            status == "failed",
        ))
    }
}

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
    workdir: Option<String>,
    timeout_seconds: Option<u64>,
}

fn aggregate_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(stdout));
    }
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    let text = text.trim_end();
    if text.is_empty() {
        "(no output)".to_string()
    } else {
        text.to_string()
    }
}

fn format_shell_output(exit_code: i32, duration_ms: u64, output: &str) -> String {
    format!(
        "Exit code: {exit_code}\nWall time: {:.3} seconds\nOutput:\n{output}",
        duration_ms as f64 / 1000.0
    )
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{
        LocalProcessRunnerHandle, LocalWorkspaceHandle, ToolCall, ToolExecutionContext,
        ToolExecutor,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn shell_tool_returns_codex_style_aggregated_output() {
        let root = temp_workspace("roder-shell");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
        };

        let result = tool
            .execute(context(&root), call(json!({ "command": "printf hi" })))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.text.contains("Exit code: 0"));
        assert!(result.text.contains("Output:\nhi"));
        assert_eq!(result.data["command"], "printf hi");
        assert_eq!(result.data["aggregated_output"], "hi");
        assert_eq!(result.data["status"], "completed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shell_tool_marks_nonzero_exit_as_error() {
        let root = temp_workspace("roder-shell-fail");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
        };

        let result = tool
            .execute(
                context(&root),
                call(json!({ "command": "echo nope >&2; exit 7" })),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.text.contains("Exit code: 7"));
        assert!(result.text.contains("nope"));
        assert_eq!(result.data["exit_code"], 7);
        assert_eq!(result.data["status"], "failed");

        let _ = std::fs::remove_dir_all(root);
    }

    fn call(arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call-shell".to_string(),
            name: "shell".to_string(),
            arguments,
            raw_arguments: "{}".to_string(),
            thread_id: "thread-shell".to_string(),
            turn_id: "turn-shell".to_string(),
        }
    }

    fn context(workspace: &std::path::Path) -> ToolExecutionContext {
        ToolExecutionContext::new(
            ThreadId::from("thread-shell"),
            TurnId::from("turn-shell"),
            PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
        .with_process_runner(Arc::new(LocalProcessRunnerHandle))
    }

    fn temp_workspace(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
