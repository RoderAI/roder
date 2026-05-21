use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::{
    ChecklistItem, Diagnostic, DiagnosticSeverity, Document, DocumentSummary, LineRange, Task,
    validate_document,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct ListOptions {
    pub include_index: bool,
}

pub fn parse_document(path: impl Into<PathBuf>, content: &str) -> Document {
    let path = path.into();
    let id = document_id(&path);
    let mut document = Document {
        id,
        path: path.clone(),
        title: String::new(),
        goal: String::new(),
        architecture: String::new(),
        tech_stack: String::new(),
        owned_paths: Vec::new(),
        tasks: Vec::new(),
        acceptance: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut in_tasks = false;
    let mut in_owned_paths = false;
    let mut in_acceptance = false;
    let mut current_task: Option<usize> = None;
    let mut in_run_block_for_task: Option<usize> = None;
    let mut run_buffer = Vec::new();
    let mut seen_task_ids = HashSet::new();
    let lines = content.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        let line_no = index + 1;
        if document.title.is_empty()
            && let Some(title) = line.strip_prefix("# ")
        {
            document.title = title.trim().to_string();
        }
        if let Some(value) = bold_field(line, "Goal") {
            document.goal = value.to_string();
        }
        if let Some(value) = bold_field(line, "Architecture") {
            document.architecture = value.to_string();
        }
        if let Some(value) = bold_field(line, "Tech Stack") {
            document.tech_stack = value.to_string();
        }
        if line.trim() == "## Tasks" {
            in_tasks = true;
            in_owned_paths = false;
            in_acceptance = false;
            continue;
        }
        if line.trim() == "## Owned Paths" {
            in_tasks = false;
            in_owned_paths = true;
            in_acceptance = false;
            continue;
        }
        if line.trim() == "## Phase Acceptance" || line.trim() == "## Final Roadmap Acceptance" {
            finish_run_block(
                &mut document.tasks,
                &mut in_run_block_for_task,
                &mut run_buffer,
            );
            in_tasks = false;
            in_owned_paths = false;
            in_acceptance = true;
            current_task = None;
            continue;
        }
        if in_owned_paths && line.starts_with("## ") {
            in_owned_paths = false;
        }
        if in_owned_paths && let Some(path) = file_path_bullet(line) {
            document.owned_paths.push(path.to_string());
            continue;
        }
        if in_tasks && line.starts_with("## ") && line.trim() != "## Tasks" {
            finish_run_block(
                &mut document.tasks,
                &mut in_run_block_for_task,
                &mut run_buffer,
            );
            current_task = None;
        }
        if in_tasks && line.trim() == "Run:" {
            in_run_block_for_task = current_task;
            run_buffer.clear();
            continue;
        }
        if in_run_block_for_task.is_some() {
            if line.starts_with("```") {
                continue;
            }
            if line.trim() == "Acceptance:" {
                finish_run_block(
                    &mut document.tasks,
                    &mut in_run_block_for_task,
                    &mut run_buffer,
                );
                continue;
            }
            if line.starts_with("### ") || line.starts_with("## ") {
                finish_run_block(
                    &mut document.tasks,
                    &mut in_run_block_for_task,
                    &mut run_buffer,
                );
            } else {
                run_buffer.push((*line).to_string());
                continue;
            }
        }
        if let Some((checked, text)) = checkbox(line) {
            if in_acceptance {
                let id = checklist_id(line_no, text);
                document.acceptance.push(ChecklistItem {
                    id,
                    text: text.to_string(),
                    checked,
                    line: line_no,
                });
            } else if in_tasks {
                finish_run_block(
                    &mut document.tasks,
                    &mut in_run_block_for_task,
                    &mut run_buffer,
                );
                let id = unique_task_id(&mut seen_task_ids, line_no, text);
                document.tasks.push(Task {
                    id,
                    heading: text.to_string(),
                    checked,
                    line: line_no,
                    level: line.chars().take_while(|ch| ch.is_whitespace()).count() / 2,
                    body_range: LineRange {
                        start: line_no,
                        end: line_no,
                    },
                    run_blocks: Vec::new(),
                    paths: Vec::new(),
                });
                current_task = Some(document.tasks.len() - 1);
            }
            continue;
        }
        if let Some(task_index) = current_task {
            document.tasks[task_index].body_range.end = line_no;
            if let Some(path) = file_path_bullet(line) {
                document.tasks[task_index].paths.push(path.to_string());
            }
        }
    }
    finish_run_block(
        &mut document.tasks,
        &mut in_run_block_for_task,
        &mut run_buffer,
    );
    document.diagnostics = validate_document(&document).diagnostics;
    document
}

pub fn set_task_checked(
    path: impl AsRef<Path>,
    task_id: &str,
    checked: bool,
    _evidence: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let document = parse_document(path, &content);
    let task = document
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
    let mut lines = content.split_inclusive('\n').collect::<Vec<_>>();
    let line_index = task.line.saturating_sub(1);
    let original = lines
        .get(line_index)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("task line out of range: {}", task.line))?;
    let next = if checked {
        original.replacen("- [ ]", "- [x]", 1)
    } else {
        original.replacen("- [x]", "- [ ]", 1)
    };
    lines[line_index] = &next;
    let updated = lines.join("");
    atomic_write(path, updated.as_bytes())
}

pub fn list_documents(
    workspace: impl AsRef<Path>,
    options: ListOptions,
) -> anyhow::Result<Vec<DocumentSummary>> {
    let roadmap_dir = workspace.as_ref().join("roadmap");
    let mut summaries = Vec::new();
    for entry in fs::read_dir(&roadmap_dir)
        .with_context(|| format!("read roadmap dir {}", roadmap_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if !options.include_index
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("00-"))
        {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        let document = parse_document(&path, &content);
        let checked_tasks = document.tasks.iter().filter(|task| task.checked).count();
        let unchecked_tasks = document.tasks.len().saturating_sub(checked_tasks);
        summaries.push(DocumentSummary {
            id: document.id,
            path,
            title: document.title,
            checked_tasks,
            unchecked_tasks,
        });
    }
    summaries.sort_by_key(|summary| {
        (
            phase_number(&summary.path).unwrap_or(usize::MAX),
            summary.path.clone(),
        )
    });
    Ok(summaries)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        std::process::id()
    ));
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn finish_run_block(tasks: &mut [Task], task: &mut Option<usize>, buffer: &mut Vec<String>) {
    if let Some(task_index) = task.take() {
        let run = buffer.join("\n").trim().to_string();
        if !run.is_empty() {
            tasks[task_index].run_blocks.push(run);
        }
        buffer.clear();
    }
}

fn bold_field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.strip_prefix(&format!("**{name}:**"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn checkbox(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    if let Some(text) = trimmed.strip_prefix("- [ ] ") {
        Some((false, text.trim()))
    } else {
        trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
            .map(|text| (true, text.trim()))
    }
}

fn unique_task_id(seen: &mut HashSet<String>, _line: usize, text: &str) -> String {
    let base = format!("task-{}", slug(text));
    if seen.insert(base.clone()) {
        return base;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if seen.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn checklist_id(line: usize, text: &str) -> String {
    format!("acceptance-{}-{}", line, slug(text))
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(64).collect()
}

fn document_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("roadmap")
        .to_string()
}

fn file_path_bullet(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix("- Create: `")
        .or_else(|| line.trim().strip_prefix("- Modify: `"))
        .and_then(|rest| rest.strip_suffix('`'))
}

fn phase_number(path: &Path) -> Option<usize> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('-').next())
        .and_then(|number| number.parse().ok())
}

fn invalid_path(path: &Path) -> Option<Diagnostic> {
    let is_roadmap_file = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("roadmap")
        && path.extension().and_then(|ext| ext.to_str()) == Some("md");
    (!is_roadmap_file).then(|| Diagnostic {
        path: path.to_path_buf(),
        line: None,
        severity: DiagnosticSeverity::Error,
        message: "roadmap documents must live under roadmap/*.md".to_string(),
    })
}

pub(crate) fn path_diagnostic(path: &Path) -> Option<Diagnostic> {
    invalid_path(path)
}
