use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::hunks::{EditHunk, lines_hunk};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexPatchChange {
    pub op: CodexPatchOp,
    pub path: String,
    pub move_to: Option<String>,
    pub lines: Vec<String>,
    pub hunks: Vec<CodexPatchHunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexPatchOp {
    Add,
    Delete,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexPatchHunk {
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}

pub fn is_codex_patch(patch: &str) -> bool {
    patch.trim_start().starts_with("*** Begin Patch")
}

pub fn codex_patch_hunks(patch: &str) -> anyhow::Result<Vec<EditHunk>> {
    let changes = parse_codex_patch(patch)?;
    let mut records = Vec::new();
    for change in changes {
        let path = change
            .move_to
            .as_deref()
            .unwrap_or(&change.path)
            .to_string();
        match change.op {
            CodexPatchOp::Add => {
                records.push(lines_hunk(path, Vec::new(), change.lines, records.len()))
            }
            CodexPatchOp::Delete => {
                records.push(lines_hunk(path, Vec::new(), Vec::new(), records.len()))
            }
            CodexPatchOp::Update => {
                for hunk in change.hunks {
                    records.push(lines_hunk(
                        path.clone(),
                        hunk.old_lines,
                        hunk.new_lines,
                        records.len(),
                    ));
                }
            }
        }
    }
    Ok(records)
}

pub fn parse_codex_patch(patch: &str) -> anyhow::Result<Vec<CodexPatchChange>> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines = normalized.split('\n').collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch") {
        bail!("missing *** Begin Patch");
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
                    bail!(
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
        bail!("unexpected patch line {:?}", line);
    }
    bail!("missing *** End Patch")
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
        bail!("{}: expected hunk header, got {:?}", change.path, line);
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
            bail!("empty hunk line must be prefixed with space, +, or -");
        }
        let body = &line[1..];
        match line.as_bytes()[0] {
            b' ' => {
                hunk.old_lines.push(body.to_string());
                hunk.new_lines.push(body.to_string());
            }
            b'-' => hunk.old_lines.push(body.to_string()),
            b'+' => hunk.new_lines.push(body.to_string()),
            prefix => bail!("invalid hunk line prefix {:?}", prefix as char),
        }
        i += 1;
    }
    if hunk.old_lines.is_empty() && hunk.new_lines.is_empty() {
        bail!("empty hunk");
    }
    Ok((hunk, i))
}

pub fn apply_codex_patch_to_workspace(root: &Path, patch: &str) -> anyhow::Result<String> {
    apply_codex_patch_to_workspace_with_external_paths(root, patch, false)
}

pub fn apply_codex_patch_to_workspace_with_external_paths(
    root: &Path,
    patch: &str,
    allow_external_paths: bool,
) -> anyhow::Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
    let changes = parse_codex_patch(patch)?;
    if changes.is_empty() {
        bail!("no changes found");
    }
    let mut summaries = Vec::new();
    for change in changes {
        summaries.push(apply_codex_patch_change(
            &root,
            change,
            allow_external_paths,
        )?);
    }
    Ok(format!("Success. {}", summaries.join("\n")))
}

fn apply_codex_patch_change(
    root: &Path,
    change: CodexPatchChange,
    allow_external_paths: bool,
) -> anyhow::Result<String> {
    let path = resolve_for_write(root, &change.path, allow_external_paths)?;
    match change.op {
        CodexPatchOp::Add => add_file(root, &path, &change),
        CodexPatchOp::Delete => delete_file(root, &path),
        CodexPatchOp::Update => update_file(root, &path, &change, allow_external_paths),
    }
}

fn add_file(root: &Path, path: &Path, change: &CodexPatchChange) -> anyhow::Result<String> {
    if path.exists() {
        bail!("{} already exists", change.path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, join_patch_lines(&change.lines))?;
    Ok(format!("Added {}", display(root, path)))
}

fn delete_file(root: &Path, path: &Path) -> anyhow::Result<String> {
    std::fs::remove_file(path)?;
    Ok(format!("Deleted {}", display(root, path)))
}

fn update_file(
    root: &Path,
    path: &PathBuf,
    change: &CodexPatchChange,
    allow_external_paths: bool,
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
            bail!("expected hunk not found in {}:\n{}", change.path, old_text);
        };
        text.replace_range(index..index + old_text.len(), &new_text);
    }

    let target_path = if let Some(move_to) = &change.move_to {
        resolve_for_write(root, move_to, allow_external_paths)?
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
            display(root, path),
            display(root, &target_path)
        ))
    } else {
        Ok(format!("Updated {}", display(root, path)))
    }
}

fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn resolve_for_write(
    root: &Path,
    input: &str,
    allow_external_paths: bool,
) -> anyhow::Result<PathBuf> {
    let candidate = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        root.join(input)
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
    let normalized = normalize(candidate)?;
    let normalized_for_check = if normalized.exists() {
        normalized.canonicalize()?
    } else if let Some(parent) = normalized.parent().filter(|parent| parent.exists()) {
        parent.canonicalize()?.join(
            normalized
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path is required"))?,
        )
    } else {
        normalized.clone()
    };
    if !allow_external_paths && !normalized_for_check.starts_with(&root) {
        bail!(
            "path {} is outside workspace {}",
            normalized_for_check.display(),
            root.display()
        );
    }
    Ok(normalized_for_check)
}

fn normalize(path: PathBuf) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("path escapes filesystem root");
                }
            }
        }
    }
    Ok(normalized)
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_applies_codex_patch() {
        let root = temp_dir("roder-edit-core-patch");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "old\n").unwrap();
        let output = apply_codex_patch_to_workspace(
            &root,
            "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch\n",
        )
        .unwrap();
        assert!(output.contains("Updated a.txt"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "new\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_paths_outside_workspace() {
        let root = temp_dir("roder-edit-core-outside");
        std::fs::create_dir_all(&root).unwrap();
        let err = apply_codex_patch_to_workspace(
            &root,
            "*** Begin Patch\n*** Add File: ../x.txt\n+no\n*** End Patch\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("outside workspace"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn can_allow_paths_outside_workspace() {
        let root = temp_dir("roder-edit-core-allow-root");
        let outside = temp_dir("roder-edit-core-allow-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let target = outside.join("x.txt");
        let output = apply_codex_patch_to_workspace_with_external_paths(
            &root,
            &format!(
                "*** Begin Patch\n*** Add File: {}\n+yes\n*** End Patch\n",
                target.display()
            ),
            true,
        )
        .unwrap();

        assert!(output.contains("Success. Added"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "yes\n");
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
