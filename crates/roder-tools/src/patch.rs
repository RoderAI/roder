use std::path::Path;
use std::process::Stdio;

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
    roder_edit_core::patch::apply_codex_patch_to_workspace(workspace.root(), patch)
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
