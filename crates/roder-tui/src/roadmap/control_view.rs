use ratatui::text::{Line, Span, Text};
use roder_roadmap::ThreadAttachment;

use super::RoadmapModeState;

pub(super) fn render_control_text(state: &RoadmapModeState) -> Text<'static> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::raw("Roadmap Control Surface "),
        Span::raw(state.label()),
    ]));
    lines.push(Line::from(""));
    render_documents(state, &mut lines);
    lines.push(Line::from(""));
    render_task_queue(state, &mut lines);
    lines.push(Line::from(""));
    render_selected_task(state, &mut lines);
    lines.push(Line::from(""));
    render_agents(state, &mut lines);
    lines.push(Line::from(""));
    render_validation(state, &mut lines);
    lines.push(Line::from(""));
    render_operator_surface(state, &mut lines);
    Text::from(lines)
}

fn render_documents(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    let total_checked = state
        .documents
        .iter()
        .map(|document| document.checked_tasks)
        .sum::<usize>();
    let total_tasks = state
        .documents
        .iter()
        .map(|document| document.checked_tasks + document.unchecked_tasks)
        .sum::<usize>();
    lines.push(Line::from(format!(
        "Plans  {}/{} tasks complete",
        total_checked, total_tasks
    )));
    if state.documents.is_empty() {
        lines.push(Line::from("  no roadmap documents found"));
        return;
    }
    for document in state.documents.iter().take(8) {
        let marker = if state
            .selected_plan
            .as_deref()
            .is_some_and(|path| document.path.ends_with(path))
        {
            ">"
        } else {
            " "
        };
        lines.push(Line::from(format!(
            "{marker} {}/{}  {}",
            document.checked_tasks,
            document.checked_tasks + document.unchecked_tasks,
            document.title
        )));
    }
}

fn render_task_queue(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from("Task Queue"));
    let Some(document) = state.selected_document.as_ref() else {
        lines.push(Line::from("  select or create a roadmap document"));
        return;
    };
    if document.tasks.is_empty() {
        lines.push(Line::from("  no tasks in selected roadmap"));
        return;
    }
    for task in document.tasks.iter().take(12) {
        let marker = if state.focused_task_id.as_deref() == Some(task.id.as_str()) {
            ">"
        } else {
            " "
        };
        let status = task_status_label(state, task.id.as_str(), task.checked);
        lines.push(Line::from(format!(
            "{marker} {status:<8} {}  {}",
            task.id, task.heading
        )));
    }
}

fn render_selected_task(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from("Selected Task"));
    let Some(document) = state.selected_document.as_ref() else {
        lines.push(Line::from("  none"));
        return;
    };
    let Some(task) = state
        .focused_task_id
        .as_deref()
        .and_then(|id| document.tasks.iter().find(|task| task.id == id))
    else {
        lines.push(Line::from("  none"));
        return;
    };
    let worker_count = threads_for_task(&state.attached_threads, &task.id).count();
    lines.push(Line::from(format!("  {}  {}", task.id, task.heading)));
    lines.push(Line::from(format!("  workers: {worker_count}")));
    if !task.paths.is_empty() {
        lines.push(Line::from(format!("  paths: {}", task.paths.join(", "))));
    }
    if let Some(run) = task.run_blocks.first() {
        lines.push(Line::from(format!("  run: {}", one_line(run))));
    }
}

fn render_agents(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from("Agent Lanes"));
    if state.attached_threads.is_empty() {
        lines.push(Line::from("  no attached roadmap workers"));
        return;
    }
    for thread in state.attached_threads.iter().take(8) {
        let marker = if state.selected_thread_id.as_deref() == Some(&thread.thread_id) {
            ">"
        } else {
            " "
        };
        lines.push(Line::from(format!(
            "{marker} {}  task={}  status={}",
            thread.thread_id,
            thread.task_id.as_deref().unwrap_or("-"),
            thread.status.as_deref().unwrap_or("-")
        )));
    }
}

fn render_validation(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from("Validation Gate"));
    if state.validation_diagnostics.is_empty() {
        lines.push(Line::from("  ok"));
        return;
    }
    for diagnostic in state.validation_diagnostics.iter().take(6) {
        lines.push(Line::from(format!(
            "  {:?}: {}",
            diagnostic.severity, diagnostic.message
        )));
    }
}

fn render_operator_surface(state: &RoadmapModeState, lines: &mut Vec<Line<'static>>) {
    let plan = state
        .selected_plan
        .as_deref()
        .unwrap_or("roadmap/<plan>.md");
    let task = state.focused_task_id.as_deref().unwrap_or("<task-id>");
    lines.push(Line::from("Operator Surface"));
    lines.push(Line::from(format!(
        "  roder roadmap dispatch {plan} {task}"
    )));
    lines.push(Line::from(format!("  roder roadmap spawn {plan} {task}")));
    lines.push(Line::from(format!(
        "  roder roadmap check {plan} {task} --done --evidence <text>"
    )));
}

fn task_status_label(state: &RoadmapModeState, task_id: &str, checked: bool) -> &'static str {
    if checked {
        "done"
    } else if threads_for_task(&state.attached_threads, task_id)
        .next()
        .is_some()
    {
        "assigned"
    } else if state.focused_task_id.as_deref() == Some(task_id) {
        "ready"
    } else {
        "pending"
    }
}

fn threads_for_task<'a>(
    threads: &'a [ThreadAttachment],
    task_id: &'a str,
) -> impl Iterator<Item = &'a ThreadAttachment> {
    threads
        .iter()
        .filter(move |thread| thread.task_id.as_deref() == Some(task_id))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
