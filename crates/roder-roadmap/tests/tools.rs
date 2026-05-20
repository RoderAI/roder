use std::fs;
use std::path::{Path, PathBuf};

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    LocalWorkspaceHandle, ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry,
};
use roder_roadmap::{RoadmapToolActivation, RoadmapToolContributor};
use serde_json::json;
use std::sync::Arc;

#[test]
fn contributor_registers_tools_only_when_activated() {
    let workspace = temp_workspace("roadmap-tools-inactive");
    let mut registry = ToolRegistry::default();
    RoadmapToolContributor::new(
        &workspace,
        workspace.join(".data"),
        RoadmapToolActivation::Inactive,
    )
    .contribute(&mut registry)
    .unwrap();
    assert!(registry.specs().is_empty());

    RoadmapToolContributor::new(
        &workspace,
        workspace.join(".data"),
        RoadmapToolActivation::ExplicitRequest,
    )
    .contribute(&mut registry)
    .unwrap();
    assert_eq!(
        registry
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        vec![
            "roadmap_create",
            "roadmap_list",
            "roadmap_patch",
            "roadmap_read",
            "roadmap_set_task_state",
            "roadmap_thread_attach",
            "roadmap_thread_list",
            "roadmap_thread_spawn",
            "roadmap_validate",
        ]
    );
}

#[tokio::test]
async fn roadmap_tools_cover_document_lifecycle_and_thread_attachments() {
    let workspace = temp_workspace("roadmap-tools-lifecycle");
    fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
    let registry = registry(&workspace);

    let listed = run_tool(&registry, &workspace, "roadmap_list", json!({})).await;
    assert_eq!(listed.data["documents"].as_array().unwrap().len(), 1);

    let read = run_tool(
        &registry,
        &workspace,
        "roadmap_read",
        json!({ "path": "roadmap/20-roadmapping-mode.md" }),
    )
    .await;
    assert_eq!(
        read.data["document"]["title"],
        "Roadmapping Mode Implementation Plan"
    );

    let validation = run_tool(
        &registry,
        &workspace,
        "roadmap_validate",
        json!({ "path": "roadmap/20-roadmapping-mode.md" }),
    )
    .await;
    assert!(
        validation.data["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let task_id = validation.data["next_unchecked_task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let missing_evidence = run_tool(
        &registry,
        &workspace,
        "roadmap_set_task_state",
        json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "task_id": task_id,
            "checked": true
        }),
    )
    .await;
    assert!(missing_evidence.is_error);
    assert!(missing_evidence.text.contains("evidence is required"));

    let checked = run_tool(
        &registry,
        &workspace,
        "roadmap_set_task_state",
        json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "task_id": task_id,
            "checked": true,
            "evidence": "tool test evidence"
        }),
    )
    .await;
    assert!(!checked.is_error);
    assert_eq!(checked.data["task_id"], task_id);
    assert!(
        fs::read_to_string(workspace.join("roadmap/20-roadmapping-mode.md"))
            .unwrap()
            .contains("- [x] Add roadmap tool tests")
    );

    let patched = run_tool(
        &registry,
        &workspace,
        "roadmap_patch",
        json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "old_string": "Runtime behavior is covered.",
            "new_string": "Roadmap tool behavior is covered."
        }),
    )
    .await;
    assert_eq!(patched.data["replacements"], 1);

    let attached = run_tool(
        &registry,
        &workspace,
        "roadmap_thread_attach",
        json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "task_id": task_id,
            "thread_id": "thread-existing",
            "title": "Existing worker"
        }),
    )
    .await;
    assert_eq!(attached.data["thread"]["thread_id"], "thread-existing");

    let spawned = run_tool(
        &registry,
        &workspace,
        "roadmap_thread_spawn",
        json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "task_id": task_id
        }),
    )
    .await;
    assert!(
        spawned.data["thread"]["thread_id"]
            .as_str()
            .unwrap()
            .starts_with("thread-")
    );

    let threads = run_tool(
        &registry,
        &workspace,
        "roadmap_thread_list",
        json!({ "path": "roadmap/20-roadmapping-mode.md" }),
    )
    .await;
    assert_eq!(threads.data["threads"].as_array().unwrap().len(), 2);

    let created = run_tool(
        &registry,
        &workspace,
        "roadmap_create",
        json!({
            "slug": "new-roadmap",
            "title": "New Roadmap",
            "goal": "Create a second roadmap."
        }),
    )
    .await;
    assert_eq!(
        created.data["document"]["title"],
        "New Roadmap Implementation Plan"
    );
    assert!(workspace.join("roadmap/21-new-roadmap.md").exists());
}

#[tokio::test]
async fn roadmap_write_tools_reject_paths_outside_roadmap_and_skill_scope() {
    let workspace = temp_workspace("roadmap-tools-scope");
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/lib.rs"), "old").unwrap();
    let registry = registry(&workspace);

    let result = run_tool(
        &registry,
        &workspace,
        "roadmap_patch",
        json!({
            "path": "src/lib.rs",
            "old_string": "old",
            "new_string": "new"
        }),
    )
    .await;

    assert!(result.is_error);
    assert!(result.text.contains("roadmap write tools are limited"));
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).unwrap(),
        "old"
    );

    let traversal = run_tool(
        &registry,
        &workspace,
        "roadmap_patch",
        json!({
            "path": "roadmap/../src/lib.rs",
            "old_string": "old",
            "new_string": "new"
        }),
    )
    .await;
    assert!(traversal.is_error);
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).unwrap(),
        "old"
    );
}

fn registry(workspace: &Path) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    RoadmapToolContributor::new(
        workspace,
        workspace.join(".data"),
        RoadmapToolActivation::RoadmappingMode,
    )
    .contribute(&mut registry)
    .unwrap();
    registry
}

async fn run_tool(
    registry: &ToolRegistry,
    workspace: &Path,
    name: &str,
    arguments: serde_json::Value,
) -> roder_api::tools::ToolResult {
    let tool = registry.get(name).unwrap();
    match tool
        .execute(
            ToolExecutionContext::new("thread-a", "turn-a", PolicyMode::Default)
                .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace))),
            ToolCall {
                id: format!("call-{name}"),
                name: name.to_string(),
                raw_arguments: arguments.to_string(),
                arguments,
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
            },
        )
        .await
    {
        Ok(result) => result,
        Err(err) => roder_api::tools::ToolResult {
            id: format!("call-{name}"),
            name: name.to_string(),
            text: err.to_string(),
            data: json!({ "error": err.to_string() }),
            is_error: true,
        },
    }
}

fn fixture() -> String {
    "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add a document-first roadmapping mode.\n**Architecture:** Roadmap Markdown documents are primary state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `crates/roder-roadmap/src/tools.rs`\n\n## Tasks\n\n- [ ] Add roadmap tool tests\n\nRun:\n\n```sh\ncargo test -p roder-roadmap --test tools\n```\n\nAcceptance:\n- Runtime behavior is covered.\n\n## Phase Acceptance\n\n- [ ] Tools work.\n".to_string()
}

fn temp_workspace(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(path.join("roadmap")).unwrap();
    path
}
