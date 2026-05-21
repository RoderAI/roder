use std::fs;
use std::path::{Path, PathBuf};

use roder_roadmap::{
    ListOptions, RoadmapRuntime, list_documents, parse_document, validate_document,
};
use serde_json::json;

pub(crate) async fn run_roadmap_cli(args: &[String]) -> anyhow::Result<()> {
    let workspace = std::env::current_dir()?;
    let mut out = String::new();
    run_roadmap_cli_with_workspace(args, &workspace, &mut out)?;
    print!("{out}");
    Ok(())
}

pub(crate) fn run_roadmap_cli_with_workspace(
    args: &[String],
    workspace: &Path,
    out: &mut String,
) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => roadmap_list(args, workspace, out),
        Some("new") => roadmap_new(args, workspace, out),
        Some("open") => roadmap_open(args, workspace, out),
        Some("status") => roadmap_status(args, workspace, out),
        Some("next") => roadmap_next(args, workspace, out),
        Some("check") => roadmap_check(args, workspace, out),
        Some("threads") => roadmap_threads(args, workspace, out),
        Some("attach") => roadmap_attach(args, workspace, out),
        Some("validate") => roadmap_validate(args, workspace, out),
        _ => anyhow::bail!(
            "usage: roder roadmap <list|new|open|status|next|check|threads|attach|validate>"
        ),
    }
}

fn roadmap_list(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let documents = list_documents(workspace, ListOptions::default())?;
    if args.iter().any(|arg| arg == "--json") {
        push_json(out, json!({ "documents": documents }))?;
    } else {
        for document in documents {
            out.push_str(&format!(
                "{}\t{}\t{}/{}\n",
                rel(workspace, &document.path),
                document.title,
                document.checked_tasks,
                document.checked_tasks + document.unchecked_tasks
            ));
        }
    }
    Ok(())
}

fn roadmap_new(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let Some(slug) = args.get(1) else {
        anyhow::bail!("usage: roder roadmap new <slug> --title <title> [--goal <goal>]");
    };
    let title = flag_value(args, "--title")
        .ok_or_else(|| anyhow::anyhow!("roder roadmap new requires --title <title>"))?;
    let goal = flag_value(args, "--goal").unwrap_or("Describe the intended outcome.");
    let slug = sanitize_slug(slug)?;
    let roadmap_dir = workspace.join("roadmap");
    fs::create_dir_all(&roadmap_dir)?;
    let phase = next_phase_number(&roadmap_dir)?;
    let path = roadmap_dir.join(format!("{phase:02}-{slug}.md"));
    if path.exists() {
        anyhow::bail!("roadmap already exists: {}", path.display());
    }
    let content = roadmap_template(title, goal, phase, &slug);
    fs::write(&path, content)?;
    update_phase_map(workspace, phase, &path)?;
    out.push_str(&format!("created {}\n", rel(workspace, &path)));
    Ok(())
}

fn roadmap_open(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let path = resolve_plan_path(workspace, plan_arg(args, 1)?)?;
    let mut runtime = runtime(workspace);
    let document = runtime.open_roadmap(&path)?;
    out.push_str(&format!(
        "opened {} for roadmapping mode\n",
        rel(workspace, &document.path)
    ));
    Ok(())
}

fn roadmap_status(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let document = read_plan(workspace, plan_arg(args, 1)?)?;
    let checked = document.tasks.iter().filter(|task| task.checked).count();
    let unchecked = document.tasks.len().saturating_sub(checked);
    let focused = document
        .tasks
        .iter()
        .find(|task| !task.checked)
        .map(|task| task.id.as_str())
        .unwrap_or("-");
    if args.iter().any(|arg| arg == "--json") {
        push_json(
            out,
            json!({ "document": document, "checked": checked, "unchecked": unchecked, "focused_task_id": focused }),
        )?;
    } else {
        out.push_str(&format!(
            "{}\tchecked={checked}\tunchecked={unchecked}\tfocused={focused}\n",
            document.title
        ));
    }
    Ok(())
}

fn roadmap_next(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let document = read_plan(workspace, plan_arg(args, 1)?)?;
    let Some(task) = document.tasks.iter().find(|task| !task.checked) else {
        out.push_str("no unchecked tasks\n");
        return Ok(());
    };
    out.push_str(&format!("{}\t{}\n", task.id, task.heading));
    Ok(())
}

fn roadmap_check(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let path = resolve_plan_path(workspace, plan_arg(args, 1)?)?;
    let Some(task_id) = args.get(2) else {
        anyhow::bail!(
            "usage: roder roadmap check <plan> <task-id> --done|--open --evidence <text>"
        );
    };
    let checked = if args.iter().any(|arg| arg == "--done") {
        true
    } else if args.iter().any(|arg| arg == "--open") {
        false
    } else {
        anyhow::bail!("roder roadmap check requires --done or --open");
    };
    let evidence = flag_value(args, "--evidence").unwrap_or_default();
    if checked && evidence.trim().is_empty() {
        anyhow::bail!("--evidence is required when marking a task done");
    }
    let mut runtime = runtime(workspace);
    runtime.set_roadmap_task(&path, task_id, checked, evidence)?;
    out.push_str(&format!("updated {task_id}\n"));
    Ok(())
}

fn roadmap_threads(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let runtime = runtime(workspace);
    let path = resolve_plan_path(workspace, plan_arg(args, 1)?)?;
    let threads = runtime.list_roadmap_threads(&path)?;
    for thread in threads {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            thread.thread_id,
            thread.task_id.unwrap_or_default(),
            thread.title.unwrap_or_default()
        ));
    }
    Ok(())
}

fn roadmap_attach(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let path = resolve_plan_path(workspace, plan_arg(args, 1)?)?;
    let Some(thread_id) = args.get(2) else {
        anyhow::bail!("usage: roder roadmap attach <plan> <thread-id> [--task <task-id>]");
    };
    let document = read_document(&path)?;
    let task_id = flag_value(args, "--task")
        .map(str::to_string)
        .or_else(|| {
            document
                .tasks
                .iter()
                .find(|task| !task.checked)
                .map(|task| task.id.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("no task available to attach"))?;
    let mut runtime = runtime(workspace);
    runtime.attach_roadmap_thread(&path, &task_id, thread_id, None)?;
    out.push_str(&format!("attached {thread_id} to {task_id}\n"));
    Ok(())
}

fn roadmap_validate(args: &[String], workspace: &Path, out: &mut String) -> anyhow::Result<()> {
    let paths = if let Some(path) = args.get(1) {
        vec![resolve_plan_path(workspace, path)?]
    } else {
        list_documents(workspace, ListOptions::default())?
            .into_iter()
            .map(|summary| summary.path)
            .collect()
    };
    let mut total = 0;
    for path in paths {
        let document = read_document(&path)?;
        let validation = validate_document(&document);
        total += validation.diagnostics.len();
        out.push_str(&format!(
            "{}\tdiagnostics={}\n",
            rel(workspace, &path),
            validation.diagnostics.len()
        ));
    }
    if total > 0 {
        anyhow::bail!("{total} roadmap diagnostics found");
    }
    Ok(())
}

fn runtime(workspace: &Path) -> RoadmapRuntime {
    RoadmapRuntime::new(workspace, workspace.join(".roder"))
}

fn read_plan(workspace: &Path, path: &str) -> anyhow::Result<roder_roadmap::Document> {
    read_document(&resolve_plan_path(workspace, path)?)
}

fn read_document(path: &Path) -> anyhow::Result<roder_roadmap::Document> {
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

fn plan_arg(args: &[String], index: usize) -> anyhow::Result<&str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("roadmap plan path is required"))
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

fn sanitize_slug(slug: &str) -> anyhow::Result<String> {
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        anyhow::bail!("slug must contain only lowercase letters, digits, and hyphens");
    }
    Ok(slug.to_string())
}

fn next_phase_number(roadmap_dir: &Path) -> anyhow::Result<u32> {
    let mut max_phase = 0;
    for entry in fs::read_dir(roadmap_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some((prefix, _)) = name.split_once('-')
            && let Ok(phase) = prefix.parse::<u32>()
        {
            max_phase = max_phase.max(phase);
        }
    }
    Ok(max_phase + 1)
}

fn roadmap_template(title: &str, goal: &str, phase: u32, slug: &str) -> String {
    format!(
        "# {title} Implementation Plan\n\n**Goal:** {goal}\n**Architecture:** Document the architecture before implementation.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `roadmap/{phase:02}-{slug}.md`\n\n## Tasks\n\n- [ ] Draft the implementation plan\n\nRun:\n\n```sh\ncargo test -p roder-roadmap\n```\n\nAcceptance:\n- The roadmap is actionable and validated.\n\n## Phase Acceptance\n\n- [ ] Plan is complete.\n"
    )
}

fn update_phase_map(workspace: &Path, phase: u32, path: &Path) -> anyhow::Result<()> {
    let index = workspace.join("roadmap/00-feature-inventory-and-sequencing.md");
    if !index.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&index)?;
    let plan = rel(workspace, path);
    let row = format!("| {phase} | `{plan}` | Roadmap agent | TBD |\n");
    if content.contains(&format!("| {phase} |")) {
        return Ok(());
    }
    let mut output = String::new();
    let mut inserted = false;
    for line in content.lines() {
        if !inserted
            && let Some(existing) = table_phase(line)
            && existing > phase
        {
            output.push_str(&row);
            inserted = true;
        }
        output.push_str(line);
        output.push('\n');
    }
    if !inserted {
        output.push_str(&row);
    }
    fs::write(index, output)?;
    Ok(())
}

fn table_phase(line: &str) -> Option<u32> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return None;
    }
    trimmed.split('|').nth(1)?.trim().parse().ok()
}

fn rel(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn push_json(out: &mut String, value: serde_json::Value) -> anyhow::Result<()> {
    out.push_str(&serde_json::to_string_pretty(&value)?);
    out.push('\n');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roadmap_cli_lifecycle_is_testable_without_tui() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("roadmap/00-feature-inventory-and-sequencing.md"),
            index(),
        )
        .unwrap();
        fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
        let mut out = String::new();

        run_roadmap_cli_with_workspace(&["list".into()], &workspace, &mut out).unwrap();
        assert!(out.contains("20-roadmapping-mode.md"));

        out.clear();
        run_roadmap_cli_with_workspace(
            &[
                "new".into(),
                "new-plan".into(),
                "--title".into(),
                "New Plan".into(),
            ],
            &workspace,
            &mut out,
        )
        .unwrap();
        assert!(workspace.join("roadmap/21-new-plan.md").exists());
        assert!(
            fs::read_to_string(workspace.join("roadmap/00-feature-inventory-and-sequencing.md"))
                .unwrap()
                .contains("| 21 | `roadmap/21-new-plan.md`")
        );

        out.clear();
        run_roadmap_cli_with_workspace(
            &[
                "status".into(),
                "20-roadmapping-mode.md".into(),
                "--json".into(),
            ],
            &workspace,
            &mut out,
        )
        .unwrap();
        assert!(out.contains("\"unchecked\": 1"));

        out.clear();
        run_roadmap_cli_with_workspace(
            &["next".into(), "20-roadmapping-mode.md".into()],
            &workspace,
            &mut out,
        )
        .unwrap();
        let task_id = out.split('\t').next().unwrap().to_string();

        out.clear();
        let before_check = fs::read_to_string(workspace.join("roadmap/20-roadmapping-mode.md"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        run_roadmap_cli_with_workspace(
            &[
                "check".into(),
                "20-roadmapping-mode.md".into(),
                task_id.clone(),
                "--done".into(),
                "--evidence".into(),
                "cli evidence".into(),
            ],
            &workspace,
            &mut out,
        )
        .unwrap();
        let after_check = fs::read_to_string(workspace.join("roadmap/20-roadmapping-mode.md"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let changed_lines = before_check
            .iter()
            .zip(after_check.iter())
            .filter(|(before, after)| before != after)
            .collect::<Vec<_>>();
        assert_eq!(changed_lines.len(), 1);
        assert_eq!(changed_lines[0].0, "- [ ] Add CLI tests");
        assert_eq!(changed_lines[0].1, "- [x] Add CLI tests");

        out.clear();
        run_roadmap_cli_with_workspace(
            &[
                "attach".into(),
                "20-roadmapping-mode.md".into(),
                "thread-a".into(),
                "--task".into(),
                task_id,
            ],
            &workspace,
            &mut out,
        )
        .unwrap();
        out.clear();
        run_roadmap_cli_with_workspace(
            &["threads".into(), "20-roadmapping-mode.md".into()],
            &workspace,
            &mut out,
        )
        .unwrap();
        assert!(out.contains("thread-a"));

        out.clear();
        run_roadmap_cli_with_workspace(
            &["validate".into(), "20-roadmapping-mode.md".into()],
            &workspace,
            &mut out,
        )
        .unwrap();
        assert!(out.contains("diagnostics=0"));

        out.clear();
        run_roadmap_cli_with_workspace(&["validate".into()], &workspace, &mut out).unwrap();
        assert!(out.contains("roadmap/20-roadmapping-mode.md"));
        assert!(out.contains("roadmap/21-new-plan.md"));
    }

    #[test]
    fn roadmap_check_done_requires_evidence() {
        let workspace = temp_workspace();
        fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
        let mut out = String::new();
        let err = run_roadmap_cli_with_workspace(
            &[
                "check".into(),
                "20-roadmapping-mode.md".into(),
                "add-cli-tests".into(),
                "--done".into(),
            ],
            &workspace,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--evidence is required"));
    }

    fn temp_workspace() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roadmap-cli-{unique}"));
        fs::create_dir_all(path.join("roadmap")).unwrap();
        path
    }

    fn fixture() -> String {
        "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add a document-first roadmapping mode.\n**Architecture:** Roadmap Markdown documents are primary state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `crates/roder-cli/src/roadmap_cli.rs`\n\n## Tasks\n\n- [ ] Add CLI tests\n\nRun:\n\n```sh\ncargo test -p roder-cli roadmap_cli\n```\n\nAcceptance:\n- CLI behavior is covered.\n\n## Phase Acceptance\n\n- [ ] CLI works.\n".to_string()
    }

    fn index() -> String {
        "# Roadmap Index\n\n## Phase Map\n\n| Phase | Plan | Primary Owner | Depends On |\n| 20 | `roadmap/20-roadmapping-mode.md` | Roadmap agent | TBD |\n| 22 | `roadmap/22-roder-web-search-extensions.md` | Search agent | TBD |\n".to_string()
    }
}
