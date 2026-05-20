use std::fs;
use std::path::PathBuf;

use roder_roadmap::{
    RoadmapEventKind, RoadmapPromptInput, RoadmapRuntime, parse_document, roadmap_context_prompt,
};

#[test]
fn runtime_opens_focuses_validates_toggles_and_publishes_events() {
    let workspace = temp_workspace();
    let data_dir = workspace.join(".data");
    let path = workspace.join("roadmap/20-roadmapping-mode.md");
    fs::write(&path, fixture()).unwrap();
    let mut runtime = RoadmapRuntime::new(&workspace, &data_dir);

    let document = runtime
        .open_roadmap("roadmap/20-roadmapping-mode.md")
        .unwrap();
    let task_id = document.tasks[0].id.clone();
    runtime
        .focus_roadmap_task("roadmap/20-roadmapping-mode.md", &task_id)
        .unwrap();
    let validation = runtime
        .validate_roadmap("roadmap/20-roadmapping-mode.md")
        .unwrap();
    assert!(validation.diagnostics.is_empty());
    runtime
        .set_roadmap_task(
            "roadmap/20-roadmapping-mode.md",
            &task_id,
            true,
            "unit test evidence",
        )
        .unwrap();
    runtime
        .record_mode_changed("roadmap/20-roadmapping-mode.md")
        .unwrap();

    let updated = fs::read_to_string(&path).unwrap();
    assert!(updated.contains("- [x] Add runtime tests"));
    assert_eq!(
        runtime
            .events()
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![
            RoadmapEventKind::Opened,
            RoadmapEventKind::TaskFocused,
            RoadmapEventKind::Validated,
            RoadmapEventKind::TaskChecked,
            RoadmapEventKind::ModeChanged,
        ]
    );
}

#[test]
fn runtime_lists_spawns_and_attaches_threads_without_transcript_mutation() {
    let workspace = temp_workspace();
    let data_dir = workspace.join(".data");
    fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
    let mut runtime = RoadmapRuntime::new(&workspace, &data_dir);
    let document = runtime
        .open_roadmap("roadmap/20-roadmapping-mode.md")
        .unwrap();
    let task_id = document.tasks[0].id.clone();

    let attached = runtime
        .attach_roadmap_thread(
            "roadmap/20-roadmapping-mode.md",
            &task_id,
            "thread-existing",
            Some("Existing worker".to_string()),
        )
        .unwrap();
    assert_eq!(attached.thread_id, "thread-existing");
    let spawned = runtime
        .spawn_roadmap_thread("roadmap/20-roadmapping-mode.md", &task_id)
        .unwrap();
    assert!(spawned.thread_id.starts_with("thread-"));

    let threads = runtime
        .list_roadmap_threads("roadmap/20-roadmapping-mode.md")
        .unwrap();
    assert_eq!(threads.len(), 2);
    assert!(runtime.events().iter().any(|event| {
        event.kind == RoadmapEventKind::ThreadAttached
            && event.thread_id.as_deref() == Some("thread-existing")
    }));
    assert!(
        runtime
            .events()
            .iter()
            .any(|event| event.kind == RoadmapEventKind::ThreadSpawned)
    );
}

#[test]
fn roadmap_prompt_includes_document_task_validation_and_skill_body() {
    let path = PathBuf::from("roadmap/20-roadmapping-mode.md");
    let document = parse_document(&path, &fixture());
    let validation = roder_roadmap::validate_document(&document);
    let skill_body = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/roadmap-planning/SKILL.md"),
    )
    .unwrap();
    let prompt = roadmap_context_prompt(RoadmapPromptInput {
        document: &document,
        focused_task: document.tasks.first(),
        validation: Some(&validation),
        skill_body: Some(&skill_body),
    });

    assert!(prompt.contains("roadmapping mode"));
    assert!(prompt.contains("Document: Roadmapping Mode Implementation Plan"));
    assert!(prompt.contains("Focused task: Add runtime tests [open]"));
    assert!(prompt.contains("Validation: no diagnostics."));
    assert!(prompt.contains("Roadmap planning skill:"));
    assert!(prompt.contains("Keep the roadmap document as the source of truth."));
}

fn fixture() -> String {
    "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add a document-first roadmapping mode.\n**Architecture:** Roadmap Markdown documents are primary state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `crates/roder-roadmap/src/runtime.rs`\n\n## Tasks\n\n- [ ] Add runtime tests\n\nRun:\n\n```sh\ncargo test -p roder-roadmap\n```\n\nAcceptance:\n- Runtime behavior is covered.\n\n## Phase Acceptance\n\n- [ ] Runtime works.\n".to_string()
}

fn temp_workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!("roadmap-runtime-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(path.join("roadmap")).unwrap();
    path
}
