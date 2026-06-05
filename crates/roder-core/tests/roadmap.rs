use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::HostedWebSearchConfig;
use roder_api::policy_mode::PolicyMode;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig, default_instructions};

#[tokio::test]
async fn core_roadmap_methods_persist_state_and_publish_events() {
    let workspace = temp_workspace();
    fs::write(workspace.join("roadmap/20-roadmapping-mode.md"), fixture()).unwrap();
    let runtime = runtime_for(&workspace);
    let mut events = runtime.subscribe_events();

    let documents = runtime.list_roadmaps().await.unwrap();
    assert_eq!(documents.len(), 1);

    let document = runtime
        .open_roadmap("roadmap/20-roadmapping-mode.md")
        .await
        .unwrap();
    let task_id = document.tasks[0].id.clone();
    runtime
        .focus_roadmap_task("roadmap/20-roadmapping-mode.md", &task_id)
        .await
        .unwrap();
    runtime
        .set_roadmap_task(
            "roadmap/20-roadmapping-mode.md",
            &task_id,
            true,
            "core integration evidence",
        )
        .await
        .unwrap();
    let validation = runtime
        .validate_roadmap("roadmap/20-roadmapping-mode.md")
        .await
        .unwrap();
    runtime
        .attach_roadmap_thread(
            "roadmap/20-roadmapping-mode.md",
            &task_id,
            "thread-existing",
            Some("Existing worker".to_string()),
        )
        .await
        .unwrap();
    runtime
        .spawn_roadmap_thread("roadmap/20-roadmapping-mode.md", &task_id)
        .await
        .unwrap();
    runtime
        .enter_roadmap_mode("roadmap/20-roadmapping-mode.md")
        .await
        .unwrap();

    assert!(validation.diagnostics.is_empty());
    assert!(
        fs::read_to_string(workspace.join("roadmap/20-roadmapping-mode.md"))
            .unwrap()
            .contains("- [x] Add core integration tests")
    );
    assert_eq!(
        runtime
            .list_roadmap_threads("roadmap/20-roadmapping-mode.md")
            .await
            .unwrap()
            .len(),
        2
    );

    let mut roadmap_events = Vec::new();
    while roadmap_events.len() < 7 {
        let envelope = events.recv().await.unwrap();
        if let RoderEvent::RoadmapChanged(event) = envelope.event {
            roadmap_events.push(event.event_kind);
        }
    }
    assert_eq!(
        roadmap_events,
        vec![
            "opened",
            "task_focused",
            "task_checked",
            "validated",
            "thread_attached",
            "thread_attached",
            "thread_spawned",
        ]
    );

    let envelope = events.recv().await.unwrap();
    assert!(matches!(
        envelope.event,
        RoderEvent::RoadmapChanged(event) if event.event_kind == "mode_changed"
    ));

    let restarted = runtime_for(&workspace);
    assert_eq!(
        restarted
            .list_roadmap_threads("roadmap/20-roadmapping-mode.md")
            .await
            .unwrap()
            .len(),
        2
    );
    restarted
        .open_roadmap("roadmap/20-roadmapping-mode.md")
        .await
        .unwrap();
    assert_eq!(
        restarted
            .list_roadmap_threads("roadmap/20-roadmapping-mode.md")
            .await
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn normal_thread_instructions_do_not_include_roadmap_context() {
    let instructions = default_instructions();
    let system = instructions.system.unwrap_or_default();
    assert!(!system.contains("Roder roadmapping mode"));
    assert!(!system.contains("Roadmap planning skill:"));
}

fn runtime_for(workspace: &std::path::Path) -> Runtime {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    Runtime::new(
        builder.build().unwrap(),
        RuntimeConfig {
            default_provider: PROVIDER_MOCK.to_string(),
            default_model: "mock".to_string(),
            reasoning: None,
            auto_compact_token_limit: None,
            file_backed_dynamic_context: false,
            hosted_web_search: HostedWebSearchConfig::disabled(),
            model_edit_tools: std::collections::HashMap::new(),
            model_parallel_tool_calls: std::collections::HashMap::new(),
            model_profiles: std::collections::HashMap::new(),
            tool_allowlist: Vec::new(),
            command_shell: roder_api::command_shell::default_command_shell(),
            workspace: Some(workspace.display().to_string()),
            policy_mode: PolicyMode::Default,
            runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
            speed_policy: Default::default(),
            dynamic_workflows: Default::default(),
            reliability: Default::default(),
            turn_deadline_seconds: None,
            remote_runner_destination: None,
            team_data_dir: Some(workspace.join(".teams")),
            roadmap_data_dir: Some(workspace.join(".data")),
            ..RuntimeConfig::default()
        },
    )
    .unwrap()
}

fn fixture() -> String {
    "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add a document-first roadmapping mode.\n**Architecture:** Roadmap Markdown documents are primary state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `crates/roder-core/src/roadmap.rs`\n\n## Tasks\n\n- [ ] Add core integration tests\n\nRun:\n\n```sh\ncargo test -p roder-core --test roadmap\n```\n\nAcceptance:\n- Runtime behavior is covered.\n\n## Phase Acceptance\n\n- [ ] Runtime works.\n".to_string()
}

fn temp_workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!("core-roadmap-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(path.join("roadmap")).unwrap();
    path
}
