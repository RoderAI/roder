use std::fs;
use std::path::PathBuf;

use roder_roadmap::{
    DiagnosticSeverity, ListOptions, RoadmapState, RoadmapStateStore, ThreadAttachment,
    list_documents, parse_document, set_task_checked, validate_document,
};
use time::OffsetDateTime;

#[test]
fn parse_existing_roadmap_extracts_core_document_shape() {
    let path = PathBuf::from("roadmap/20-roadmapping-mode.md");
    let path = workspace_root().join(path);
    let content = fs::read_to_string(&path).unwrap();
    let document = parse_document(&path, &content);

    assert_eq!(document.title, "Roadmapping Mode Implementation Plan");
    assert!(document.goal.contains("document-first roadmapping mode"));
    assert!(document.architecture.contains("roadmap Markdown documents"));
    assert!(document.tech_stack.contains("JSON"));
    assert!(
        document
            .owned_paths
            .iter()
            .any(|path| path.contains("roadmap"))
    );
    assert!(document.tasks.iter().any(|task| {
        task.heading
            .contains("Add tests that parse existing roadmap files")
            && task.checked
            && task.line > 0
            && !task.id.is_empty()
    }));
    assert!(
        document
            .tasks
            .iter()
            .any(|task| !task.run_blocks.is_empty())
    );
    assert!(
        document
            .acceptance
            .iter()
            .any(|item| { item.text.contains("Normal thread sessions are unchanged") })
    );
}

#[test]
fn task_ids_remain_stable_when_unrelated_lines_change() {
    let path = PathBuf::from("roadmap/20-roadmapping-mode.md");
    let path = workspace_root().join(path);
    let content = fs::read_to_string(&path).unwrap();
    let edited = content.replace(
        "**Goal:** Add a document-first roadmapping mode",
        "**Goal:** Add a durable document-first roadmapping mode\n\n<!-- unrelated edit -->",
    );

    let original = parse_document(&path, &content);
    let changed = parse_document(&path, &edited);

    assert_eq!(
        original
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>(),
        changed
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn checkbox_update_preserves_every_other_byte() {
    let dir = temp_dir("roadmap-checkbox");
    let roadmap_dir = dir.join("roadmap");
    fs::create_dir_all(&roadmap_dir).unwrap();
    let path = roadmap_dir.join("99-test.md");
    fs::write(&path, fixture(false)).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let document = parse_document(&path, &original);
    let task_id = document.tasks[0].id.clone();

    set_task_checked(&path, &task_id, true, "unit test evidence").unwrap();

    let updated = fs::read_to_string(&path).unwrap();
    let original_lines = original.lines().collect::<Vec<_>>();
    let updated_lines = updated.lines().collect::<Vec<_>>();
    assert_eq!(original_lines.len(), updated_lines.len());
    for (index, (before, after)) in original_lines.iter().zip(updated_lines.iter()).enumerate() {
        if index + 1 == document.tasks[0].line {
            assert_eq!(*after, "- [x] First task");
        } else {
            assert_eq!(before, after);
        }
    }
}

#[test]
fn validation_reports_structural_failures_with_paths_and_lines() {
    let path = PathBuf::from("notes/not-roadmap.txt");
    let document = parse_document(&path, "# Broken\n\n- [ ] Floating task\n");
    let result = validate_document(&document);

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == DiagnosticSeverity::Error
            && diagnostic.path == path
            && diagnostic.message.contains("roadmap/*.md")
    }));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Goal"))
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Architecture"))
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("owned paths"))
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Run"))
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("acceptance"))
    );
}

#[test]
fn validation_reports_duplicate_task_ids() {
    let mut document = parse_document(
        "roadmap/99-duplicate.md",
        "# Test Plan\n\n**Goal:** Test.\n**Architecture:** Test.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `src/lib.rs`\n\n## Tasks\n\n- [ ] First task\n- [ ] Second task\n\nRun:\n\n```sh\ncargo test\n```\n\nAcceptance:\n- Works.\n\n## Phase Acceptance\n\n- [ ] Done.\n",
    );
    document.tasks[1].id = document.tasks[0].id.clone();

    let result = validate_document(&document);

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.line == Some(document.tasks[1].line)
            && diagnostic.message.contains("duplicate task id")
    }));
}

#[test]
fn list_documents_excludes_index_and_sorts_by_phase() {
    let dir = temp_dir("roadmap-list");
    let roadmap_dir = dir.join("roadmap");
    fs::create_dir_all(&roadmap_dir).unwrap();
    fs::write(roadmap_dir.join("00-index.md"), fixture(true)).unwrap();
    fs::write(roadmap_dir.join("20-alpha.md"), fixture(true)).unwrap();
    fs::write(roadmap_dir.join("03-beta.md"), fixture(true)).unwrap();

    let documents = list_documents(&dir, ListOptions::default()).unwrap();

    assert_eq!(
        documents
            .iter()
            .map(|summary| summary.path.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["03-beta.md", "20-alpha.md"]
    );
}

#[test]
fn state_store_round_trips_with_atomic_path() {
    let dir = temp_dir("roadmap-store");
    let store = RoadmapStateStore::new(&dir);
    let now = OffsetDateTime::now_utc();
    let state = RoadmapState {
        document_id: "20-roadmapping-mode".to_string(),
        path: PathBuf::from("roadmap/20-roadmapping-mode.md"),
        focused_task_id: Some("task-1".to_string()),
        primary_thread_id: Some("thread-primary".to_string()),
        attached_thread_id: Some("thread-attached".to_string()),
        threads: vec![ThreadAttachment {
            thread_id: "thread-attached".to_string(),
            task_id: Some("task-1".to_string()),
            title: Some("worker".to_string()),
            status: Some("active".to_string()),
            created_at: now,
            updated_at: now,
        }],
        last_validation: Some(now),
        last_diagnostics: Vec::new(),
        updated_at: now,
    };

    assert!(store.load().unwrap().is_none());
    store.save(&state).unwrap();

    assert_eq!(store.path(), dir.join("roadmaps").join("state.json"));
    assert_eq!(store.load().unwrap(), Some(state));
}

fn fixture(checked: bool) -> String {
    let mark = if checked { "x" } else { " " };
    format!(
        "# Test Plan\n\n**Goal:** Test goal.\n**Architecture:** Test architecture.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `src/lib.rs`\n\n## Tasks\n\n- [{mark}] First task\n\nRun:\n\n```sh\ncargo test\n```\n\nAcceptance:\n- First task works.\n\n## Phase Acceptance\n\n- [{mark}] Plan works.\n"
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf()
}
