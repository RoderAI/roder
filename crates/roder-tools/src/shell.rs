use std::sync::Arc;
use std::time::Instant;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use crate::command_shell::shell_for_context;
use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    command_shell: String,
) -> anyhow::Result<()> {
    registry.register(Arc::new(ShellTool {
        workspace,
        command_shell,
    }))
}

#[derive(Debug)]
struct ShellTool {
    workspace: Workspace,
    command_shell: String,
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
            Some(workdir) => workspace.resolve_existing_workdir(workdir)?,
            _ => workspace.root().to_path_buf(),
        };
        let timeout = args
            .timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, 600);
        let shell = shell_for_context(&ctx, &self.command_shell);
        let started = Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            Command::new(&shell)
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
                "shell": shell,
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
            command_shell: roder_api::command_shell::default_command_shell(),
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
            command_shell: roder_api::command_shell::default_command_shell(),
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

    #[tokio::test]
    async fn shell_tool_accepts_workspace_root_workdir_variants() {
        let root = temp_workspace("roder-shell-workdir");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
        };

        for workdir in [".", "./", " . /", "'.'", "` . / `"] {
            let result = tool
                .execute(
                    context(&root),
                    call(json!({ "command": "printf ok", "workdir": workdir })),
                )
                .await
                .unwrap();
            assert!(!result.is_error, "workdir {workdir:?}: {}", result.text);
            assert_eq!(result.data["status"], "completed");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_tool_uses_context_command_shell() {
        let root = temp_workspace("roder-shell-context-shell");
        std::fs::create_dir_all(&root).unwrap();
        let shell = root.join("record-shell.sh");
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf '%s\\n' \"$0\" > used-shell.txt\nexec /bin/sh \"$@\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&shell).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shell, permissions).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: "bash".to_string(),
        };

        let result = tool
            .execute(
                context(&root).with_command_shell(shell.display().to_string()),
                call(json!({ "command": "printf ok" })),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["shell"], shell.display().to_string());
        assert_eq!(
            std::fs::read_to_string(root.join("used-shell.txt"))
                .unwrap()
                .trim(),
            shell.display().to_string()
        );

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
