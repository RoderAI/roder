use std::path::{Path, PathBuf};
use std::process::Stdio;

use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::files::{parse, result};
use crate::workspace::Workspace;

#[derive(Debug)]
pub(crate) struct ApplyPatchTool {
    pub(crate) workspace: Workspace,
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ApplyPatchArgs>(&call)?;
        if args.patch.trim().is_empty() {
            return Ok(result(
                call,
                "failed to apply patch: patch is required".to_string(),
                json!({ "error": { "kind": "empty_patch" } }),
                true,
            ));
        }

        let outcome = if is_codex_patch(&args.patch) {
            apply_codex_patch(&self.workspace, &args.patch).await
        } else {
            apply_unified_patch(&self.workspace, &args.patch).await
        };

        match outcome {
            Ok(text) => Ok(result(call, text, json!({}), false)),
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

#[derive(Debug)]
struct CodexPatchChange {
    op: CodexPatchOp,
    path: String,
    move_to: Option<String>,
    lines: Vec<String>,
    hunks: Vec<CodexPatchHunk>,
}

#[derive(Debug)]
enum CodexPatchOp {
    Add,
    Delete,
    Update,
}

#[derive(Debug)]
struct CodexPatchHunk {
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn is_codex_patch(patch: &str) -> bool {
    patch.trim_start().starts_with("*** Begin Patch")
}

async fn apply_unified_patch(workspace: &Workspace, patch: &str) -> anyhow::Result<String> {
    validate_unified_patch_paths(workspace, patch)?;
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
    let changes = parse_codex_patch(patch)?;
    if changes.is_empty() {
        anyhow::bail!("no changes found");
    }

    let mut summaries = Vec::new();
    for change in changes {
        summaries.push(apply_codex_patch_change(workspace, change)?);
    }
    Ok(format!("Success. {}", summaries.join("\n")))
}

fn parse_codex_patch(patch: &str) -> anyhow::Result<Vec<CodexPatchChange>> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines = normalized.split('\n').collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch") {
        anyhow::bail!("missing *** Begin Patch");
    }

    let mut changes = Vec::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i];
        if line == "*** End Patch" {
            return Ok(changes);
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let mut change = CodexPatchChange {
                op: CodexPatchOp::Add,
                path: path.trim().to_string(),
                move_to: None,
                lines: Vec::new(),
                hunks: Vec::new(),
            };
            i += 1;
            while i < lines.len() && !lines[i].starts_with("*** ") {
                let Some(line) = lines[i].strip_prefix('+') else {
                    anyhow::bail!(
                        "add file {} contains non-add line {:?}",
                        change.path,
                        lines[i]
                    );
                };
                change.lines.push(line.to_string());
                i += 1;
            }
            changes.push(change);
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            changes.push(CodexPatchChange {
                op: CodexPatchOp::Delete,
                path: path.trim().to_string(),
                move_to: None,
                lines: Vec::new(),
                hunks: Vec::new(),
            });
            i += 1;
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let (change, next) = parse_codex_update(path.trim(), &lines, i + 1)?;
            changes.push(change);
            i = next;
            continue;
        }
        anyhow::bail!("unexpected patch line {:?}", line);
    }
    anyhow::bail!("missing *** End Patch")
}

fn parse_codex_update(
    path: &str,
    lines: &[&str],
    mut i: usize,
) -> anyhow::Result<(CodexPatchChange, usize)> {
    let mut change = CodexPatchChange {
        op: CodexPatchOp::Update,
        path: path.to_string(),
        move_to: None,
        lines: Vec::new(),
        hunks: Vec::new(),
    };
    while i < lines.len() {
        let line = lines[i];
        if line == "*** End Patch"
            || line.starts_with("*** Add File: ")
            || line.starts_with("*** Delete File: ")
            || line.starts_with("*** Update File: ")
        {
            return Ok((change, i));
        }
        if let Some(path) = line.strip_prefix("*** Move to: ") {
            change.move_to = Some(path.trim().to_string());
            i += 1;
            continue;
        }
        if line.starts_with("@@") {
            let (hunk, next) = parse_codex_patch_hunk(lines, i + 1)
                .map_err(|err| anyhow::anyhow!("{}: {err}", change.path))?;
            change.hunks.push(hunk);
            i = next;
            continue;
        }
        anyhow::bail!("{}: expected hunk header, got {:?}", change.path, line);
    }
    Ok((change, i))
}

fn parse_codex_patch_hunk(lines: &[&str], mut i: usize) -> anyhow::Result<(CodexPatchHunk, usize)> {
    let mut hunk = CodexPatchHunk {
        old_lines: Vec::new(),
        new_lines: Vec::new(),
    };
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("@@") || line.starts_with("*** ") {
            break;
        }
        if line == "*** End of File" {
            i += 1;
            continue;
        }
        if line.is_empty() {
            anyhow::bail!("empty hunk line must be prefixed with space, +, or -");
        }
        let body = &line[1..];
        match line.as_bytes()[0] {
            b' ' => {
                hunk.old_lines.push(body.to_string());
                hunk.new_lines.push(body.to_string());
            }
            b'-' => hunk.old_lines.push(body.to_string()),
            b'+' => hunk.new_lines.push(body.to_string()),
            prefix => anyhow::bail!("invalid hunk line prefix {:?}", prefix as char),
        }
        i += 1;
    }
    if hunk.old_lines.is_empty() && hunk.new_lines.is_empty() {
        anyhow::bail!("empty hunk");
    }
    Ok((hunk, i))
}

fn apply_codex_patch_change(
    workspace: &Workspace,
    change: CodexPatchChange,
) -> anyhow::Result<String> {
    let path = workspace.resolve_for_write(&change.path)?;
    match change.op {
        CodexPatchOp::Add => add_file(workspace, &path, &change),
        CodexPatchOp::Delete => delete_file(workspace, &path),
        CodexPatchOp::Update => update_file(workspace, &path, &change),
    }
}

fn add_file(
    workspace: &Workspace,
    path: &Path,
    change: &CodexPatchChange,
) -> anyhow::Result<String> {
    if path.exists() {
        anyhow::bail!("{} already exists", change.path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, join_patch_lines(&change.lines))?;
    Ok(format!("Added {}", workspace.display(path)))
}

fn delete_file(workspace: &Workspace, path: &Path) -> anyhow::Result<String> {
    std::fs::remove_file(path)?;
    Ok(format!("Deleted {}", workspace.display(path)))
}

fn update_file(
    workspace: &Workspace,
    path: &PathBuf,
    change: &CodexPatchChange,
) -> anyhow::Result<String> {
    let mut text = std::fs::read_to_string(path)?;
    for hunk in &change.hunks {
        let old_text = hunk.old_lines.join("\n");
        let new_text = hunk.new_lines.join("\n");
        if old_text.is_empty() {
            text = format!("{new_text}{text}");
            continue;
        }
        let Some(index) = text.find(&old_text) else {
            anyhow::bail!("expected hunk not found in {}:\n{}", change.path, old_text);
        };
        text.replace_range(index..index + old_text.len(), &new_text);
    }

    let target_path = if let Some(move_to) = &change.move_to {
        workspace.resolve_for_write(move_to)?
    } else {
        path.clone()
    };
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target_path, text)?;
    if target_path != *path {
        std::fs::remove_file(path)?;
        Ok(format!(
            "Moved {} to {}",
            workspace.display(path),
            workspace.display(&target_path)
        ))
    } else {
        Ok(format!("Updated {}", workspace.display(path)))
    }
}

fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}
