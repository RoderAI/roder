use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    Diagnostic, Document, DocumentSummary, ListOptions, RoadmapStateStore, Task, ThreadAttachment,
    list_documents, parse_document, validate_document,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapControlSnapshot {
    pub documents: Vec<DocumentSummary>,
    pub selected: Option<RoadmapDocumentControl>,
    pub total_checked_tasks: usize,
    pub total_unchecked_tasks: usize,
    pub next_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapDocumentControl {
    pub path: PathBuf,
    pub title: String,
    pub goal: String,
    pub checked_tasks: usize,
    pub unchecked_tasks: usize,
    pub focused_task_id: Option<String>,
    pub selected_thread_id: Option<String>,
    pub tasks: Vec<RoadmapTaskControl>,
    pub threads: Vec<ThreadAttachment>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapTaskControl {
    pub id: String,
    pub heading: String,
    pub checked: bool,
    pub line: usize,
    pub level: usize,
    pub status: RoadmapTaskStatus,
    pub paths: Vec<String>,
    pub run_blocks: Vec<String>,
    pub threads: Vec<ThreadAttachment>,
    pub recommended_action: String,
    pub dispatch_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoadmapTaskStatus {
    Done,
    Ready,
    Assigned,
    Pending,
}

pub fn build_control_snapshot(
    workspace: impl AsRef<Path>,
    data_dir: impl AsRef<Path>,
    selected_plan: Option<&str>,
) -> anyhow::Result<RoadmapControlSnapshot> {
    let workspace = workspace.as_ref();
    let documents = list_documents(workspace, ListOptions::default())?;
    let total_checked_tasks = documents
        .iter()
        .map(|document| document.checked_tasks)
        .sum();
    let total_unchecked_tasks = documents
        .iter()
        .map(|document| document.unchecked_tasks)
        .sum();
    let selected_path = selected_plan
        .map(|plan| resolve_plan_path(workspace, plan))
        .transpose()?
        .or_else(|| documents.first().map(|document| document.path.clone()));
    let selected = selected_path
        .as_deref()
        .map(|path| build_document_control(workspace, data_dir.as_ref(), path))
        .transpose()?;
    let next_action = next_action(selected.as_ref(), total_unchecked_tasks);

    Ok(RoadmapControlSnapshot {
        documents,
        selected,
        total_checked_tasks,
        total_unchecked_tasks,
        next_action,
    })
}

pub fn dispatch_prompt(document: &Document, task: &Task) -> String {
    dispatch_prompt_with_path(document, task, &document.path.display().to_string())
}

fn dispatch_prompt_with_path(document: &Document, task: &Task, path_label: &str) -> String {
    let mut prompt = format!(
        "You are a Roder roadmap worker.\n\
         Execute the focused roadmap task and keep the roadmap Markdown file as source of truth.\n\n\
         Roadmap: {}\n\
         Title: {}\n\
         Goal: {}\n\n\
         Task: {}\n\
         Task ID: {}\n",
        path_label, document.title, document.goal, task.heading, task.id
    );

    if !task.paths.is_empty() {
        prompt.push_str("\nOwned or task-local paths:\n");
        for path in &task.paths {
            prompt.push_str(&format!("- {path}\n"));
        }
    }
    if !task.run_blocks.is_empty() {
        prompt.push_str("\nRun commands:\n");
        for run in &task.run_blocks {
            prompt.push_str("```sh\n");
            prompt.push_str(run);
            prompt.push_str("\n```\n");
        }
    }
    prompt.push_str(
        "\nCompletion rule: only mark the task done after the stated acceptance criteria and run commands are satisfied, then record evidence.",
    );
    prompt
}

fn build_document_control(
    workspace: &Path,
    data_dir: &Path,
    path: &Path,
) -> anyhow::Result<RoadmapDocumentControl> {
    let content = fs::read_to_string(path)?;
    let document = parse_document(path, &content);
    let state = RoadmapStateStore::new(data_dir)
        .load()?
        .filter(|state| state.path == path);
    let threads = state
        .as_ref()
        .map(|state| state.threads.clone())
        .unwrap_or_default();
    let focused_task_id = state
        .as_ref()
        .and_then(|state| state.focused_task_id.clone())
        .or_else(|| {
            document
                .tasks
                .iter()
                .find(|task| !task.checked)
                .or_else(|| document.tasks.first())
                .map(|task| task.id.clone())
        });
    let selected_thread_id = state
        .as_ref()
        .and_then(|state| state.attached_thread_id.clone());
    let checked_tasks = document.tasks.iter().filter(|task| task.checked).count();
    let unchecked_tasks = document.tasks.len().saturating_sub(checked_tasks);
    let diagnostics = validate_document(&document).diagnostics;
    let threads_by_task = threads_by_task(&threads);
    let tasks = document
        .tasks
        .iter()
        .map(|task| {
            let task_threads = threads_by_task.get(&task.id).cloned().unwrap_or_default();
            let status = task_status(task, &task_threads, focused_task_id.as_deref());
            RoadmapTaskControl {
                id: task.id.clone(),
                heading: task.heading.clone(),
                checked: task.checked,
                line: task.line,
                level: task.level,
                status,
                paths: task.paths.clone(),
                run_blocks: task.run_blocks.clone(),
                threads: task_threads,
                recommended_action: recommended_task_action(status),
                dispatch_prompt: dispatch_prompt_with_path(
                    &document,
                    task,
                    &rel(workspace, &document.path),
                ),
            }
        })
        .collect();

    Ok(RoadmapDocumentControl {
        path: rel(workspace, &document.path).into(),
        title: document.title,
        goal: document.goal,
        checked_tasks,
        unchecked_tasks,
        focused_task_id,
        selected_thread_id,
        tasks,
        threads,
        diagnostics,
    })
}

fn threads_by_task(threads: &[ThreadAttachment]) -> HashMap<String, Vec<ThreadAttachment>> {
    let mut by_task: HashMap<String, Vec<ThreadAttachment>> = HashMap::new();
    for thread in threads {
        if let Some(task_id) = thread.task_id.as_ref() {
            by_task
                .entry(task_id.clone())
                .or_default()
                .push(thread.clone());
        }
    }
    by_task
}

fn task_status(
    task: &Task,
    threads: &[ThreadAttachment],
    focused_task_id: Option<&str>,
) -> RoadmapTaskStatus {
    if task.checked {
        RoadmapTaskStatus::Done
    } else if !threads.is_empty() {
        RoadmapTaskStatus::Assigned
    } else if focused_task_id == Some(task.id.as_str()) {
        RoadmapTaskStatus::Ready
    } else {
        RoadmapTaskStatus::Pending
    }
}

fn recommended_task_action(status: RoadmapTaskStatus) -> String {
    match status {
        RoadmapTaskStatus::Done => "review evidence or reopen if acceptance changed",
        RoadmapTaskStatus::Ready => "dispatch a worker or continue the focused thread",
        RoadmapTaskStatus::Assigned => "inspect attached worker progress and steer if blocked",
        RoadmapTaskStatus::Pending => "wait for earlier tasks or focus this task explicitly",
    }
    .to_string()
}

fn next_action(selected: Option<&RoadmapDocumentControl>, total_unchecked_tasks: usize) -> String {
    let Some(selected) = selected else {
        return "create or select a roadmap document".to_string();
    };
    if !selected.diagnostics.is_empty() {
        return "fix roadmap validation diagnostics before dispatching workers".to_string();
    }
    if let Some(task) = selected
        .tasks
        .iter()
        .find(|task| task.status == RoadmapTaskStatus::Ready)
    {
        return format!("dispatch or continue {}", task.id);
    }
    if total_unchecked_tasks == 0 {
        "all roadmap tasks are complete".to_string()
    } else {
        "select the next unchecked roadmap task".to_string()
    }
}

fn resolve_plan_path(workspace: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let path = if path.starts_with("roadmap/") {
        workspace.join(path)
    } else if path.ends_with(".md") {
        workspace.join("roadmap").join(path)
    } else {
        workspace.join("roadmap").join(format!("{path}.md"))
    };
    if path.parent() == Some(&workspace.join("roadmap"))
        && path.extension().and_then(|ext| ext.to_str()) == Some("md")
    {
        Ok(path)
    } else {
        anyhow::bail!("plan must resolve under roadmap/*.md")
    }
}

fn rel(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}
