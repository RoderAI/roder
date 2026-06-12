use std::fs;
use std::path::{Path, PathBuf};

use ratatui::text::Text;
use roder_roadmap::{
    Diagnostic, Document, DocumentSummary, ListOptions, ThreadAttachment, list_documents,
    parse_document, validate_document,
};
use time::OffsetDateTime;

mod control_view;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RoadmapModeState {
    pub workspace: Option<PathBuf>,
    pub selected_plan: Option<String>,
    pub documents: Vec<DocumentSummary>,
    pub selected_document: Option<Document>,
    pub focused_pane: RoadmapPaneFocus,
    pub focused_task_id: Option<String>,
    pub attached_threads: Vec<ThreadAttachment>,
    pub selected_thread_id: Option<String>,
    pub validation_diagnostics: Vec<Diagnostic>,
    pub task_detail_scroll: u16,
    pub validation_scroll: u16,
    pub activity_scroll: u16,
    pub composer_text: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RoadmapPaneFocus {
    Plans,
    Tasks,
    TaskDetail,
    Agents,
    Validation,
    Activity,
}

impl RoadmapPaneFocus {
    const ORDER: [Self; 6] = [
        Self::Plans,
        Self::Tasks,
        Self::TaskDetail,
        Self::Agents,
        Self::Validation,
        Self::Activity,
    ];

    fn next(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|pane| *pane == self)
            .unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|pane| *pane == self)
            .unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plans => "plans",
            Self::Tasks => "tasks",
            Self::TaskDetail => "task",
            Self::Agents => "agents",
            Self::Validation => "validation",
            Self::Activity => "activity",
        }
    }
}

impl RoadmapModeState {
    pub fn new(selected_plan: Option<String>) -> Self {
        Self {
            workspace: None,
            selected_plan,
            documents: Vec::new(),
            selected_document: None,
            focused_pane: RoadmapPaneFocus::Tasks,
            focused_task_id: None,
            attached_threads: Vec::new(),
            selected_thread_id: None,
            validation_diagnostics: Vec::new(),
            task_detail_scroll: 0,
            validation_scroll: 0,
            activity_scroll: 0,
            composer_text: String::new(),
        }
    }

    pub fn load(workspace: &Path, selected_plan: Option<String>) -> anyhow::Result<Self> {
        let documents = list_documents(workspace, ListOptions::default())?;
        let selected_path = selected_plan
            .as_deref()
            .map(|plan| resolve_plan_path(workspace, plan))
            .transpose()?
            .or_else(|| documents.first().map(|summary| summary.path.clone()));
        let selected_document = selected_path.as_deref().map(read_document).transpose()?;
        let validation_diagnostics = selected_document
            .as_ref()
            .map(validate_document)
            .map(|result| result.diagnostics)
            .unwrap_or_default();
        let focused_task_id = selected_document.as_ref().and_then(|document| {
            document
                .tasks
                .iter()
                .find(|task| !task.checked)
                .or_else(|| document.tasks.first())
                .map(|task| task.id.clone())
        });

        Ok(Self {
            workspace: Some(workspace.to_path_buf()),
            selected_plan: selected_path.map(|path| rel(workspace, &path)),
            documents,
            selected_document,
            focused_pane: RoadmapPaneFocus::Tasks,
            focused_task_id,
            attached_threads: Vec::new(),
            selected_thread_id: None,
            validation_diagnostics,
            task_detail_scroll: 0,
            validation_scroll: 0,
            activity_scroll: 0,
            composer_text: String::new(),
        })
    }

    pub fn label(&self) -> String {
        self.selected_plan
            .as_deref()
            .and_then(|path| path.rsplit('/').next())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("select")
            .to_string()
    }

    pub fn composer_placeholder(&self) -> &'static str {
        "Edit or execute the selected roadmap plan"
    }

    pub fn focused_task_heading(&self) -> Option<&str> {
        let document = self.selected_document.as_ref()?;
        let focused = self.focused_task_id.as_deref()?;
        document
            .tasks
            .iter()
            .find(|task| task.id == focused)
            .map(|task| task.heading.as_str())
    }

    pub fn focus_next_task(&mut self) -> Option<&str> {
        let document = self.selected_document.as_ref()?;
        if document.tasks.is_empty() {
            self.focused_task_id = None;
            return None;
        }
        let current = self
            .focused_task_id
            .as_deref()
            .and_then(|id| document.tasks.iter().position(|task| task.id == id))
            .unwrap_or(0);
        let next = (current + 1).min(document.tasks.len().saturating_sub(1));
        self.focused_task_id = Some(document.tasks[next].id.clone());
        self.focused_task_id.as_deref()
    }

    pub fn focus_previous_task(&mut self) -> Option<&str> {
        let document = self.selected_document.as_ref()?;
        if document.tasks.is_empty() {
            self.focused_task_id = None;
            return None;
        }
        let current = self
            .focused_task_id
            .as_deref()
            .and_then(|id| document.tasks.iter().position(|task| task.id == id))
            .unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.focused_task_id = Some(document.tasks[previous].id.clone());
        self.focused_task_id.as_deref()
    }

    pub fn focus_next_plan(&mut self) -> anyhow::Result<Option<&str>> {
        self.focus_plan_by_delta(1)
    }

    pub fn focus_previous_plan(&mut self) -> anyhow::Result<Option<&str>> {
        self.focus_plan_by_delta(-1)
    }

    fn focus_plan_by_delta(&mut self, delta: isize) -> anyhow::Result<Option<&str>> {
        if self.documents.is_empty() {
            self.selected_plan = None;
            self.selected_document = None;
            self.focused_task_id = None;
            return Ok(None);
        }
        let current = self
            .selected_document
            .as_ref()
            .and_then(|selected| {
                self.documents
                    .iter()
                    .position(|document| document.path == selected.path)
            })
            .or_else(|| {
                self.selected_plan.as_deref().and_then(|selected| {
                    self.documents
                        .iter()
                        .position(|document| document.path.ends_with(selected))
                })
            })
            .unwrap_or(0);
        let last = self.documents.len().saturating_sub(1);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        self.select_plan_at(next)?;
        Ok(self.selected_plan.as_deref())
    }

    fn select_plan_at(&mut self, index: usize) -> anyhow::Result<()> {
        let Some(summary) = self.documents.get(index) else {
            return Ok(());
        };
        let document = read_document(&summary.path)?;
        self.selected_plan = Some(
            self.workspace
                .as_deref()
                .map(|workspace| rel(workspace, &summary.path))
                .unwrap_or_else(|| summary.path.display().to_string().replace('\\', "/")),
        );
        self.focused_task_id = document
            .tasks
            .iter()
            .find(|task| !task.checked)
            .or_else(|| document.tasks.first())
            .map(|task| task.id.clone());
        self.validation_diagnostics = validate_document(&document).diagnostics;
        self.selected_document = Some(document);
        self.task_detail_scroll = 0;
        self.validation_scroll = 0;
        Ok(())
    }

    pub fn focus_next_pane(&mut self) -> RoadmapPaneFocus {
        self.focused_pane = self.focused_pane.next();
        self.focused_pane
    }

    pub fn focus_previous_pane(&mut self) -> RoadmapPaneFocus {
        self.focused_pane = self.focused_pane.previous();
        self.focused_pane
    }

    pub fn attach_thread(&mut self, thread_id: impl Into<String>) {
        let thread_id = thread_id.into();
        let now = OffsetDateTime::now_utc();
        let attachment = ThreadAttachment {
            thread_id: thread_id.clone(),
            task_id: self.focused_task_id.clone(),
            title: Some("Roadmap thread".to_string()),
            status: Some("attached".to_string()),
            created_at: now,
            updated_at: now,
        };
        self.selected_thread_id = Some(thread_id);
        self.attached_threads.push(attachment);
    }

    pub fn detach_selected_thread(&mut self) -> Option<String> {
        let thread_id = self.selected_thread_id.take()?;
        self.attached_threads
            .retain(|thread| thread.thread_id != thread_id);
        Some(thread_id)
    }

    pub fn select_next_thread(&mut self) -> Option<&str> {
        if self.attached_threads.is_empty() {
            self.selected_thread_id = None;
            return None;
        }
        let current = self
            .selected_thread_id
            .as_deref()
            .and_then(|id| {
                self.attached_threads
                    .iter()
                    .position(|thread| thread.thread_id == id)
            })
            .unwrap_or(0);
        let next = (current + 1) % self.attached_threads.len();
        self.selected_thread_id = Some(self.attached_threads[next].thread_id.clone());
        self.selected_thread_id.as_deref()
    }

    pub fn select_previous_thread(&mut self) -> Option<&str> {
        if self.attached_threads.is_empty() {
            self.selected_thread_id = None;
            return None;
        }
        let current = self
            .selected_thread_id
            .as_deref()
            .and_then(|id| {
                self.attached_threads
                    .iter()
                    .position(|thread| thread.thread_id == id)
            })
            .unwrap_or(0);
        let previous = if current == 0 {
            self.attached_threads.len().saturating_sub(1)
        } else {
            current - 1
        };
        self.selected_thread_id = Some(self.attached_threads[previous].thread_id.clone());
        self.selected_thread_id.as_deref()
    }

    pub fn scroll_focused_pane_down(&mut self) {
        match self.focused_pane {
            RoadmapPaneFocus::TaskDetail => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_add(1);
            }
            RoadmapPaneFocus::Validation => {
                self.validation_scroll = self.validation_scroll.saturating_add(1);
            }
            RoadmapPaneFocus::Activity => {
                self.activity_scroll = self.activity_scroll.saturating_add(1);
            }
            _ => {}
        }
    }

    pub fn scroll_focused_pane_up(&mut self) {
        match self.focused_pane {
            RoadmapPaneFocus::TaskDetail => {
                self.task_detail_scroll = self.task_detail_scroll.saturating_sub(1);
            }
            RoadmapPaneFocus::Validation => {
                self.validation_scroll = self.validation_scroll.saturating_sub(1);
            }
            RoadmapPaneFocus::Activity => {
                self.activity_scroll = self.activity_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub fn validate_selected_document(&mut self) {
        self.validation_diagnostics = self
            .selected_document
            .as_ref()
            .map(validate_document)
            .map(|result| result.diagnostics)
            .unwrap_or_default();
    }

    pub fn checkbox_toggle_confirmation(&self) -> Option<String> {
        let task = self.focused_task_heading()?;
        Some(format!(
            "Toggle roadmap task '{task}'? Evidence is required before marking done."
        ))
    }

    pub fn prompt_context(&self, input: &str) -> String {
        let mut context = String::from(
            "Roadmapping mode is active. Treat the selected roadmap document as primary state.\n",
        );
        context.push_str(roder_roadmap::ORCHESTRATOR_RULES);
        context.push('\n');
        if let Some(path) = self.selected_plan.as_deref() {
            context.push_str(&format!("Selected roadmap: {path}\n"));
        }
        if let Some(task_id) = self.focused_task_id.as_deref() {
            context.push_str(&format!("Focused task: {task_id}\n"));
        }
        if let Some(heading) = self.focused_task_heading() {
            context.push_str(&format!("Focused task heading: {heading}\n"));
        }
        let workers = self.attached_threads.len();
        if workers > 0 {
            context.push_str(&format!("Active workers: {workers}\n"));
        }
        context.push_str("User request:\n");
        context.push_str(input);
        context
    }

    /// Unchecked tasks without an attached worker, in document order, capped
    /// at `limit`. These are the targets for a fan-out spawn.
    pub fn fan_out_task_ids(&self, limit: usize) -> Vec<String> {
        let Some(document) = self.selected_document.as_ref() else {
            return Vec::new();
        };
        document
            .tasks
            .iter()
            .filter(|task| !task.checked)
            .filter(|task| {
                !self
                    .attached_threads
                    .iter()
                    .any(|thread| thread.task_id.as_deref() == Some(task.id.as_str()))
            })
            .take(limit)
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn render_text(&self) -> Text<'static> {
        control_view::render_control_text(self)
    }
}

fn read_document(path: &Path) -> anyhow::Result<Document> {
    let content = fs::read_to_string(path)?;
    Ok(parse_document(path, &content))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roadmap_mode_label_uses_file_name() {
        let state = RoadmapModeState::new(Some("roadmap/20-roadmapping-mode.md".to_string()));

        assert_eq!(state.label(), "20-roadmapping-mode.md");
    }

    #[test]
    fn roadmap_mode_label_prompts_selection_without_plan() {
        let state = RoadmapModeState::new(None);

        assert_eq!(state.label(), "select");
    }

    #[test]
    fn roadmap_model_loads_document_control_surface() {
        let workspace = temp_workspace();
        fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();

        let state =
            RoadmapModeState::load(&workspace, Some("20-roadmapping-mode.md".to_string())).unwrap();

        assert_eq!(state.label(), "20-roadmapping-mode.md");
        assert_eq!(
            state.focused_task_id.as_deref(),
            Some("task-add-roadmap-tests")
        );
        let rendered = state
            .render_text()
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Plans"));
        assert!(rendered.contains("Task Queue"));
        assert!(rendered.contains("Agent Lanes"));
        assert!(rendered.contains("Validation Gate"));
        assert!(rendered.contains("Operator Surface"));
    }

    #[test]
    fn roadmap_prompt_context_keeps_document_primary() {
        let mut state = RoadmapModeState::new(Some("roadmap/20-roadmapping-mode.md".to_string()));
        state.focused_task_id = Some("task-a".to_string());
        state.attach_thread("thread-a");

        let prompt = state.prompt_context("continue");

        assert!(prompt.contains("Roadmapping mode is active"));
        assert!(prompt.contains("Orchestrator contract:"));
        assert!(prompt.contains("roadmap_thread_spawn"));
        assert!(prompt.contains("Do not implement tasks in this thread."));
        assert!(prompt.contains("Selected roadmap: roadmap/20-roadmapping-mode.md"));
        assert!(prompt.contains("Focused task: task-a"));
        assert!(prompt.contains("Active workers: 1"));
        assert!(prompt.ends_with("continue"));
    }

    #[test]
    fn roadmap_fan_out_targets_unchecked_unassigned_tasks_in_order() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("roadmap/30-fan-out.md"),
            "# Fan Out Plan\n\n**Goal:** Fan out workers.\n**Architecture:** Documents are primary state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-roadmap/src/lib.rs`\n\n## Tasks\n\n- [x] Done task\n- [ ] First open task\n- [ ] Second open task\n- [ ] Third open task\n\nRun:\n\n```sh\ncargo test -p roder-roadmap\n```\n\nAcceptance:\n- Workers fan out.\n\n## Phase Acceptance\n\n- [ ] Works.\n",
        )
        .unwrap();
        let mut state =
            RoadmapModeState::load(&workspace, Some("30-fan-out.md".to_string())).unwrap();

        let all = state.fan_out_task_ids(8);
        assert_eq!(
            all,
            vec![
                "task-first-open-task".to_string(),
                "task-second-open-task".to_string(),
                "task-third-open-task".to_string(),
            ]
        );

        // A worker on a task removes it from fan-out targets; the cap bounds
        // how many tasks a single fan-out can claim.
        state.focused_task_id = Some("task-first-open-task".to_string());
        state.attach_thread("thread-a");
        assert_eq!(
            state.fan_out_task_ids(1),
            vec!["task-second-open-task".to_string()]
        );
    }

    #[test]
    fn roadmap_model_supports_navigation_threads_validation_and_confirmation() {
        let workspace = temp_workspace();
        fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
        let mut state =
            RoadmapModeState::load(&workspace, Some("20-roadmapping-mode.md".to_string())).unwrap();

        assert_eq!(state.focus_next_task(), Some("task-wire-roadmap-keys"));
        assert_eq!(state.focus_previous_task(), Some("task-add-roadmap-tests"));
        state.attach_thread("thread-a");
        state.attach_thread("thread-b");
        assert_eq!(state.selected_thread_id.as_deref(), Some("thread-b"));
        assert_eq!(state.select_next_thread(), Some("thread-a"));
        assert_eq!(state.detach_selected_thread().as_deref(), Some("thread-a"));
        state.validate_selected_document();
        assert!(state.validation_diagnostics.is_empty());
        assert!(
            state
                .checkbox_toggle_confirmation()
                .unwrap()
                .contains("Evidence is required")
        );
    }

    #[test]
    fn roadmap_model_renders_repo_sized_state_without_stack_pressure() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf();
        let state = RoadmapModeState::load(&workspace, None).unwrap();
        let rendered = state.render_text();

        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.spans.iter().any(|span| span.content.contains("Plans")))
        );
    }

    fn temp_workspace() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roadmap-tui-{unique}"));
        fs::create_dir_all(path.join("roadmap")).unwrap();
        path
    }

    fn fixture() -> String {
        "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add roadmapping mode.\n**Architecture:** Roadmap documents are first-class state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-tui/src/roadmap.rs`\n\n## Tasks\n\n- [ ] Add roadmap tests\n- [ ] Wire roadmap keys\n\nRun:\n\n```sh\ncargo test -p roder-tui roadmap\n```\n\nAcceptance:\n- Roadmap mode renders.\n\n## Phase Acceptance\n\n- [ ] TUI works.\n".to_string()
    }
}
