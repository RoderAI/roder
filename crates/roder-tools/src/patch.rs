use std::path::Path;
use std::process::Stdio;

use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest,
};
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::backend::{WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::files::{parse, result};
use crate::hunk_output;
use crate::workspace::Workspace;

pub(crate) struct ApplyPatchTool {
    pub(crate) workspace: Workspace,
    pub(crate) backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a unified patch in the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Patch text to apply from the workspace root."
                    }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<ApplyPatchArgs>(&call)?;
        if args.patch.trim().is_empty() {
            return Ok(result(
                call,
                "failed to apply patch: patch is required".to_string(),
                json!({ "error": { "kind": "empty_patch" } }),
                true,
            ));
        }

        let hunks = hunk_records_from_patch(&ctx, &call, &args.patch).unwrap_or_default();
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let outcome = backend.apply_patch(&args.patch).await;

        match outcome {
            Ok(text) => Ok(result(call, text, json!({ "hunks": hunks }), false)),
            Err(err) => Ok(result(
                call,
                format!("failed to apply patch: {err}"),
                json!({ "error": { "kind": "apply_patch_failed", "message": err.to_string() } }),
                true,
            )),
        }
    }
}

#[derive(Deserialize)]
struct ApplyPatchArgs {
    patch: String,
}

fn is_codex_patch(patch: &str) -> bool {
    roder_edit_core::patch::is_codex_patch(patch)
}

fn hunk_records_from_patch(
    ctx: &ToolExecutionContext,
    call: &ToolCall,
    patch: &str,
) -> anyhow::Result<Vec<roder_api::plan_review::HunkRecord>> {
    if !is_codex_patch(patch) {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for hunk in roder_edit_core::patch::codex_patch_hunks(patch)? {
        records.push(hunk_output::from_core(ctx, call, records.len(), hunk));
    }
    Ok(records)
}

pub(crate) async fn apply_patch_to_workspace(
    workspace: &Workspace,
    patch: &str,
) -> anyhow::Result<String> {
    if is_codex_patch(patch) {
        apply_codex_patch(workspace, patch).await
    } else {
        apply_unified_patch(workspace, patch).await
    }
}

/**
 * Applies a patch entirely through a remote runner session so nothing touches
 * the local filesystem. Codex patches are replayed as read/write/delete
 * operations; unified patches are uploaded to a temp file inside the
 * workspace and applied with `git apply` on the runner.
 */
pub(crate) async fn apply_patch_to_runner_workspace(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    patch: &str,
) -> anyhow::Result<String> {
    if is_codex_patch(patch) {
        apply_codex_patch_via_runner(workspace, session, patch).await
    } else {
        apply_unified_patch_via_runner(workspace, session, patch).await
    }
}

async fn apply_codex_patch_via_runner(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    patch: &str,
) -> anyhow::Result<String> {
    let changes = roder_edit_core::patch::parse_codex_patch(patch)?;
    if changes.is_empty() {
        anyhow::bail!("no changes found");
    }
    let mut summaries = Vec::new();
    for change in changes {
        let rel = workspace.display(&workspace.resolve_for_write(&change.path)?);
        match change.op {
            roder_edit_core::patch::CodexPatchOp::Add => {
                if runner_path_exists(workspace, session, &rel).await? {
                    anyhow::bail!("{} already exists", change.path);
                }
                session
                    .write_file(RunnerFileWriteRequest {
                        path: rel.clone().into(),
                        contents: join_patch_lines(&change.lines).into_bytes(),
                    })
                    .await?;
                summaries.push(format!("Added {rel}"));
            }
            roder_edit_core::patch::CodexPatchOp::Delete => {
                runner_remove_file(workspace, session, &rel).await?;
                summaries.push(format!("Deleted {rel}"));
            }
            roder_edit_core::patch::CodexPatchOp::Update => {
                let read = session
                    .read_file(RunnerFileReadRequest {
                        path: rel.clone().into(),
                    })
                    .await?;
                let mut text = String::from_utf8(read.contents)?;
                for hunk in &change.hunks {
                    let old_text = hunk.old_lines.join("\n");
                    let new_text = hunk.new_lines.join("\n");
                    if old_text.is_empty() {
                        text = format!("{new_text}{text}");
                        continue;
                    }
                    let Some(index) = text.find(&old_text) else {
                        anyhow::bail!("expected hunk not found in {}:\n{old_text}", change.path);
                    };
                    text.replace_range(index..index + old_text.len(), &new_text);
                }
                let target = match &change.move_to {
                    Some(move_to) => workspace.display(&workspace.resolve_for_write(move_to)?),
                    None => rel.clone(),
                };
                session
                    .write_file(RunnerFileWriteRequest {
                        path: target.clone().into(),
                        contents: text.into_bytes(),
                    })
                    .await?;
                if target != rel {
                    runner_remove_file(workspace, session, &rel).await?;
                    summaries.push(format!("Moved {rel} to {target}"));
                } else {
                    summaries.push(format!("Updated {rel}"));
                }
            }
        }
    }
    Ok(format!("Success. {}", summaries.join("\n")))
}

async fn apply_unified_patch_via_runner(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    patch: &str,
) -> anyhow::Result<String> {
    validate_unified_patch_paths(workspace, patch)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temp_name = format!(".roder-apply-patch-{nanos}.patch");
    session
        .write_file(RunnerFileWriteRequest {
            path: temp_name.clone().into(),
            contents: patch.as_bytes().to_vec(),
        })
        .await?;
    let output = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch".to_string(),
            program: "git".to_string(),
            args: vec![
                "apply".to_string(),
                "--whitespace=nowarn".to_string(),
                temp_name.clone(),
            ],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
        })
        .await;
    let cleanup = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch-cleanup".to_string(),
            program: "rm".to_string(),
            args: vec!["-f".to_string(), "--".to_string(), temp_name],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
        })
        .await;
    let output = output?;
    cleanup?;
    let text = format!("{}{}", output.stdout, output.stderr)
        .trim()
        .to_string();
    if output.exit_code != Some(0) {
        anyhow::bail!(
            "{}",
            if text.is_empty() {
                format!("git apply exited with {:?}", output.exit_code)
            } else {
                text
            }
        );
    }
    Ok(if text.is_empty() {
        "Success. Applied patch".to_string()
    } else {
        text
    })
}

async fn runner_path_exists(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    rel: &str,
) -> anyhow::Result<bool> {
    let output = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch-stat".to_string(),
            program: "test".to_string(),
            args: vec!["-e".to_string(), rel.to_string()],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
        })
        .await?;
    Ok(output.exit_code == Some(0))
}

async fn runner_remove_file(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    rel: &str,
) -> anyhow::Result<()> {
    let output = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch-delete".to_string(),
            program: "rm".to_string(),
            args: vec!["--".to_string(), rel.to_string()],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
        })
        .await?;
    if output.exit_code != Some(0) {
        anyhow::bail!("delete {rel} failed: {}", output.stderr.trim_end());
    }
    Ok(())
}

fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

async fn apply_unified_patch(workspace: &Workspace, patch: &str) -> anyhow::Result<String> {
    validate_unified_patch_paths(workspace, patch)?;
    if workspace.path_scope().allows_external_paths() && unified_patch_has_absolute_path(patch) {
        return apply_unified_patch_with_system_patch(patch).await;
    }
    let mut child = tokio::process::Command::new("git")
        .args(["apply", "--whitespace=nowarn", "-"])
        .current_dir(workspace.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to open git apply stdin"))?;
    stdin.write_all(patch.as_bytes()).await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_string();
    if !output.status.success() {
        anyhow::bail!(
            "{}",
            if text.is_empty() {
                format!("git apply exited with {}", output.status)
            } else {
                text
            }
        );
    }
    Ok(if text.is_empty() {
        "Success. Applied patch".to_string()
    } else {
        text
    })
}

async fn apply_unified_patch_with_system_patch(patch: &str) -> anyhow::Result<String> {
    let mut child = tokio::process::Command::new("patch")
        .args(["-p0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to open patch stdin"))?;
    stdin.write_all(patch.as_bytes()).await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_string();
    if !output.status.success() {
        anyhow::bail!(
            "{}",
            if text.is_empty() {
                format!("patch exited with {}", output.status)
            } else {
                text
            }
        );
    }
    Ok(if text.is_empty() {
        "Success. Applied patch".to_string()
    } else {
        text
    })
}

fn unified_patch_has_absolute_path(patch: &str) -> bool {
    patch.lines().any(|line| {
        line.strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
            .map(|path| path.split('\t').next().unwrap_or(path).trim())
            .filter(|path| *path != "/dev/null")
            .is_some_and(|path| Path::new(strip_diff_prefix(path)).is_absolute())
    })
}

fn validate_unified_patch_paths(workspace: &Workspace, patch: &str) -> anyhow::Result<()> {
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for path in rest.split_whitespace().take(2) {
                validate_patch_path(workspace, strip_diff_prefix(path))?;
            }
        } else if let Some(path) = line.strip_prefix("--- ") {
            validate_patch_header_path(workspace, path)?;
        } else if let Some(path) = line.strip_prefix("+++ ") {
            validate_patch_header_path(workspace, path)?;
        }
    }
    Ok(())
}

fn validate_patch_header_path(workspace: &Workspace, path: &str) -> anyhow::Result<()> {
    let path = path.split('\t').next().unwrap_or(path).trim();
    if path == "/dev/null" {
        return Ok(());
    }
    validate_patch_path(workspace, strip_diff_prefix(path))
}

fn validate_patch_path(workspace: &Workspace, path: &str) -> anyhow::Result<()> {
    workspace.resolve_for_write(path).map(|_| ())
}

fn strip_diff_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

async fn apply_codex_patch(workspace: &Workspace, patch: &str) -> anyhow::Result<String> {
    roder_edit_core::patch::apply_codex_patch_to_workspace_with_external_paths(
        workspace.root(),
        patch,
        workspace.path_scope().allows_external_paths(),
    )
}

#[cfg(test)]
mod hunk_tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;
    use serde_json::json;

    #[test]
    fn codex_patch_produces_hunk_records() {
        let ctx = ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default);
        let call = ToolCall {
            id: "patch-1".to_string(),
            name: "apply_patch".to_string(),
            arguments: json!({}),
            raw_arguments: "{}".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        };
        let records = hunk_records_from_patch(
            &ctx,
            &call,
            "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n",
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, "src/lib.rs");
        assert_eq!(records[0].tool_name, "apply_patch");
        assert_eq!(records[0].diff.len(), 2);
    }
}
