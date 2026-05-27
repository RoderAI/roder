use std::path::Path;

use roder_roadmap::{
    RoadmapDocumentControl, RoadmapTaskControl, RoadmapTaskStatus, build_control_snapshot,
};
use serde_json::json;

use super::{flag_value, plan_arg, push_json, read_document, rel, resolve_plan_path, runtime};

pub(super) fn roadmap_board(
    args: &[String],
    workspace: &Path,
    out: &mut String,
) -> anyhow::Result<()> {
    let plan = args
        .get(1)
        .filter(|value| !value.starts_with("--"))
        .map(String::as_str);
    let snapshot = build_control_snapshot(workspace, workspace.join(".roder"), plan)?;
    if args.iter().any(|arg| arg == "--json") {
        push_json(out, json!(snapshot))?;
        return Ok(());
    }

    out.push_str("Roadmap control surface\n");
    out.push_str(&format!(
        "checked={}\tunchecked={}\tnext={}\n",
        snapshot.total_checked_tasks, snapshot.total_unchecked_tasks, snapshot.next_action
    ));
    out.push('\n');

    for document in &snapshot.documents {
        out.push_str(&format!(
            "{}\t{}\t{}/{}\n",
            rel(workspace, &document.path),
            document.title,
            document.checked_tasks,
            document.checked_tasks + document.unchecked_tasks
        ));
    }

    if let Some(selected) = snapshot.selected.as_ref() {
        out.push('\n');
        render_selected_document(selected, out);
    }
    Ok(())
}

pub(super) fn roadmap_dispatch(
    args: &[String],
    workspace: &Path,
    out: &mut String,
) -> anyhow::Result<()> {
    let plan = plan_arg(args, 1)?;
    let task_id = args.get(2).filter(|value| !value.starts_with("--"));
    let snapshot = build_control_snapshot(workspace, workspace.join(".roder"), Some(plan))?;
    let selected = snapshot
        .selected
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no roadmap selected"))?;
    let task = select_task(selected, task_id.map(String::as_str))?;
    if args.iter().any(|arg| arg == "--json") {
        push_json(
            out,
            json!({
                "plan": selected.path,
                "task": task,
                "dispatchPrompt": task.dispatch_prompt,
            }),
        )?;
    } else {
        out.push_str(&task.dispatch_prompt);
        out.push('\n');
    }
    Ok(())
}

pub(super) fn roadmap_spawn(
    args: &[String],
    workspace: &Path,
    out: &mut String,
) -> anyhow::Result<()> {
    let plan = plan_arg(args, 1)?;
    let path = resolve_plan_path(workspace, plan)?;
    let document = read_document(&path)?;
    let task_id = args
        .get(2)
        .filter(|value| !value.starts_with("--"))
        .map(String::as_str)
        .or_else(|| flag_value(args, "--task"))
        .or_else(|| {
            document
                .tasks
                .iter()
                .find(|task| !task.checked)
                .map(|task| task.id.as_str())
        })
        .ok_or_else(|| anyhow::anyhow!("no task available to spawn"))?;
    let mut runtime = runtime(workspace);
    let attachment = runtime.spawn_roadmap_thread(&path, task_id)?;
    if args.iter().any(|arg| arg == "--json") {
        push_json(out, json!({ "attachment": attachment }))?;
    } else {
        out.push_str(&format!(
            "spawned worker attachment {}\t{}\n",
            attachment.thread_id, task_id
        ));
    }
    Ok(())
}

fn render_selected_document(selected: &RoadmapDocumentControl, out: &mut String) {
    out.push_str(&format!(
        "Plan\t{}\t{}\tchecked={}\tunchecked={}\tdiagnostics={}\tagents={}\n",
        selected.path.display(),
        selected.title,
        selected.checked_tasks,
        selected.unchecked_tasks,
        selected.diagnostics.len(),
        selected.threads.len()
    ));
    out.push_str("Tasks\n");
    for task in &selected.tasks {
        let focus = if selected.focused_task_id.as_deref() == Some(task.id.as_str()) {
            ">"
        } else {
            " "
        };
        out.push_str(&format!(
            "{focus} {}\t{}\tagents={}\taction={}\n",
            status_label(task.status),
            task.id,
            task.threads.len(),
            task.recommended_action
        ));
        out.push_str(&format!("  {}\n", task.heading));
    }
    if !selected.threads.is_empty() {
        out.push_str("Agents\n");
        for thread in &selected.threads {
            out.push_str(&format!(
                "{}\t{}\t{}\n",
                thread.thread_id,
                thread.task_id.as_deref().unwrap_or("-"),
                thread.status.as_deref().unwrap_or("-")
            ));
        }
    }
}

fn select_task<'a>(
    selected: &'a RoadmapDocumentControl,
    task_id: Option<&str>,
) -> anyhow::Result<&'a RoadmapTaskControl> {
    if let Some(task_id) = task_id {
        return selected
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"));
    }
    selected
        .tasks
        .iter()
        .find(|task| task.status == RoadmapTaskStatus::Ready)
        .or_else(|| selected.tasks.iter().find(|task| !task.checked))
        .ok_or_else(|| anyhow::anyhow!("no unchecked task available"))
}

fn status_label(status: RoadmapTaskStatus) -> &'static str {
    match status {
        RoadmapTaskStatus::Done => "done",
        RoadmapTaskStatus::Ready => "ready",
        RoadmapTaskStatus::Assigned => "assigned",
        RoadmapTaskStatus::Pending => "pending",
    }
}
