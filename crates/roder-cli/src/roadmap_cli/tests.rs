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
fn roadmap_cli_board_dispatch_and_spawn_expose_control_surface() {
    let workspace = temp_workspace();
    fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
    let mut out = String::new();

    run_roadmap_cli_with_workspace(
        &["board".into(), "20-roadmapping-mode.md".into()],
        &workspace,
        &mut out,
    )
    .unwrap();
    assert!(out.contains("Roadmap control surface"));
    assert!(out.contains("Tasks"));
    assert!(out.contains("ready"));

    out.clear();
    run_roadmap_cli_with_workspace(
        &["dispatch".into(), "20-roadmapping-mode.md".into()],
        &workspace,
        &mut out,
    )
    .unwrap();
    assert!(out.contains("You are a Roder roadmap worker"));
    assert!(out.contains("Task ID: task-add-cli-tests"));

    out.clear();
    run_roadmap_cli_with_workspace(
        &["spawn".into(), "20-roadmapping-mode.md".into()],
        &workspace,
        &mut out,
    )
    .unwrap();
    assert!(out.contains("spawned worker attachment"));

    out.clear();
    run_roadmap_cli_with_workspace(
        &["board".into(), "20-roadmapping-mode.md".into()],
        &workspace,
        &mut out,
    )
    .unwrap();
    assert!(out.contains("assigned"));
    assert!(out.contains("Agents"));
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
