use std::fs;
use std::path::{Path, PathBuf};

use ratatui::text::{Line, Span, Text};
use roder_roadmap::{
    Diagnostic, Document, DocumentSummary, ListOptions, ThreadAttachment, list_documents,
    parse_document, validate_document,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RoadmapModeState {
    pub selected_plan: Option<String>,
    pub documents: Vec<DocumentSummary>,
    pub selected_document: Option<Document>,
    pub focused_task_id: Option<String>,
    pub attached_threads: Vec<ThreadAttachment>,
    pub selected_thread_id: Option<String>,
    pub validation_diagnostics: Vec<Diagnostic>,
    pub composer_text: String,
}

impl RoadmapModeState {
    pub fn new(selected_plan: Option<String>) -> Self {
        Self {
            selected_plan,
            documents: Vec::new(),
            selected_document: None,
            focused_task_id: None,
            attached_threads: Vec::new(),
            selected_thread_id: None,
            validation_diagnostics: Vec::new(),
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
            selected_plan: selected_path.map(|path| rel(workspace, &path)),
            documents,
            selected_document,
            focused_task_id,
            attached_threads: Vec::new(),
            selected_thread_id: None,
            validation_diagnostics,
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

    pub fn prompt_context(&self, input: &str) -> String {
        let mut context = String::from(
            "Roadmapping mode is active. Treat the selected roadmap document as primary state.\n",
        );
        if let Some(path) = self.selected_plan.as_deref() {
            context.push_str(&format!("Selected roadmap: {path}\n"));
        }
        if let Some(task_id) = self.focused_task_id.as_deref() {
            context.push_str(&format!("Focused task: {task_id}\n"));
        }
        if let Some(heading) = self.focused_task_heading() {
            context.push_str(&format!("Focused task heading: {heading}\n"));
        }
        context.push_str("User request:\n");
        context.push_str(input);
        context
    }

    pub fn render_text(&self) -> Text<'static> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::raw("Roadmap "),
            Span::raw(self.label()),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from("Documents"));
        if self.documents.is_empty() {
            lines.push(Line::from("  no roadmap documents found"));
        } else {
            for document in self.documents.iter().take(8) {
                let marker = if self
                    .selected_plan
                    .as_deref()
                    .is_some_and(|path| document.path.ends_with(path))
                {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!(
                    "{marker} {}  {}/{}",
                    document.title,
                    document.checked_tasks,
                    document.checked_tasks + document.unchecked_tasks
                )));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Outline"));
        if let Some(document) = self.selected_document.as_ref() {
            for task in document.tasks.iter().take(10) {
                let marker = if self.focused_task_id.as_deref() == Some(task.id.as_str()) {
                    ">"
                } else {
                    " "
                };
                let checkbox = if task.checked { "[x]" } else { "[ ]" };
                lines.push(Line::from(format!(
                    "{marker} {checkbox} {}  {}",
                    task.id, task.heading
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("Document"));
            lines.push(Line::from(format!("  {}", document.title)));
            lines.push(Line::from(format!("  Goal: {}", document.goal)));
            lines.push(Line::from(format!(
                "  Architecture: {}",
                document.architecture
            )));
            lines.push(Line::from(format!("  Tech Stack: {}", document.tech_stack)));
        } else {
            lines.push(Line::from("  select or create a roadmap document"));
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Focused Task"));
        lines.push(Line::from(format!(
            "  {}",
            self.focused_task_heading().unwrap_or("none")
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("Thread Pane"));
        if self.attached_threads.is_empty() {
            lines.push(Line::from("  no attached roadmap threads"));
        } else {
            for thread in self.attached_threads.iter().take(5) {
                let marker = if self.selected_thread_id.as_deref() == Some(&thread.thread_id) {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!(
                    "{marker} {}  {}",
                    thread.thread_id,
                    thread.task_id.as_deref().unwrap_or("-")
                )));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Validation"));
        if self.validation_diagnostics.is_empty() {
            lines.push(Line::from("  ok"));
        } else {
            for diagnostic in self.validation_diagnostics.iter().take(5) {
                lines.push(Line::from(format!(
                    "  {:?}: {}",
                    diagnostic.severity, diagnostic.message
                )));
            }
        }
        Text::from(lines)
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
    fn roadmap_model_loads_document_outline_and_validation() {
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
        assert!(rendered.contains("Documents"));
        assert!(rendered.contains("Outline"));
        assert!(rendered.contains("Thread Pane"));
        assert!(rendered.contains("Validation"));
    }

    #[test]
    fn roadmap_prompt_context_keeps_document_primary() {
        let mut state = RoadmapModeState::new(Some("roadmap/20-roadmapping-mode.md".to_string()));
        state.focused_task_id = Some("task-a".to_string());

        let prompt = state.prompt_context("continue");

        assert!(prompt.contains("Roadmapping mode is active"));
        assert!(prompt.contains("Selected roadmap: roadmap/20-roadmapping-mode.md"));
        assert!(prompt.contains("Focused task: task-a"));
        assert!(prompt.ends_with("continue"));
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
        "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add roadmapping mode.\n**Architecture:** Roadmap documents are first-class state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-tui/src/roadmap.rs`\n\n## Tasks\n\n- [ ] Add roadmap tests\n\nRun:\n\n```sh\ncargo test -p roder-tui roadmap\n```\n\nAcceptance:\n- Roadmap mode renders.\n\n## Phase Acceptance\n\n- [ ] TUI works.\n".to_string()
    }
}
