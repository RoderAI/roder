use super::codex_v2::CODEX_V2_MAX_RESIDENT_TEAM_THREADS;
use super::*;

use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

#[tokio::test]
async fn codex_v2_child_inherits_live_parent_model_and_ultra_reasoning() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        codex_v2_team_runtime("inherits-selection").await;
    let parent_turn_id = "active-parent-turn".to_string();
    runtime.active_turn_selections.write().await.insert(
        parent_turn_id.clone(),
        ModelSelectionMode::manual(
            roder_api::catalog::PROVIDER_CODEX,
            "gpt-5.6-terra",
            Some(REASONING_ULTRA.to_string()),
        ),
    );

    let team = runtime
        .spawn_team_member_for_caller(
            &parent_thread_id,
            &parent_turn_id,
            TeamMemberStartRequest {
                name: "Inherited child".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap();
    let child = team.members.last().unwrap();
    let metadata = runtime
        .load_thread_metadata(&child.thread_id)
        .await
        .unwrap()
        .expect("persisted child metadata");

    assert_eq!(runtime.status().await.default_provider, PROVIDER_MOCK);
    assert_eq!(runtime.status().await.default_model, "mock");
    assert_eq!(
        metadata.selection_mode,
        Some(ModelSelectionMode::manual(
            roder_api::catalog::PROVIDER_CODEX,
            "gpt-5.6-terra",
            Some(REASONING_ULTRA.to_string()),
        ))
    );
    assert_eq!(metadata.provider.as_deref(), Some("codex"));
    assert_eq!(metadata.model.as_deref(), Some("gpt-5.6-terra"));

    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_recursive_spawns_share_the_four_resident_thread_cap() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        codex_v2_team_runtime("recursive-cap").await;
    let test_turn_id = "recursive-test-turn".to_string();

    let first = runtime
        .spawn_team_member_for_caller(
            &parent_thread_id,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "First".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap();
    let first_child = first.members.last().unwrap().thread_id.clone();
    let second = runtime
        .spawn_team_member_for_caller(
            &first_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Recursive second".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap();
    let second_child = second.members.last().unwrap().thread_id.clone();
    let listed = runtime
        .execute_agent_control_tool(
            &first_child,
            &"list-turn".to_string(),
            &ToolCallCompleted {
                id: "list-from-child".to_string(),
                name: "list_agents".to_string(),
                arguments: "{}".to_string(),
            },
            serde_json::json!({}),
        )
        .await;
    assert!(!listed.is_error, "child list failed: {}", listed.text);
    assert!(
        listed.data["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| agent["thread_id"] == second_child),
        "a recursive caller must see the agent it spawned"
    );

    let third = runtime
        .spawn_team_member_for_caller(
            &second_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Recursive third".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap();
    let third_child = third.members.last().unwrap().thread_id.clone();

    assert_eq!(first.id, second.id);
    assert_eq!(second.id, third.id);
    assert_eq!(third.members.len(), CODEX_V2_MAX_RESIDENT_TEAM_THREADS);
    let error = runtime
        .spawn_team_member_for_caller(
            &third_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Over the cap".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains(
            "agent thread limit reached for this Codex V2 team: maximum 4 resident threads"
        ),
        "unexpected cap error: {error}"
    );
    let teams = runtime.list_teams().await;
    assert_eq!(teams.len(), 1, "recursive spawn must not create a new team");
    assert_eq!(teams[0].members.len(), CODEX_V2_MAX_RESIDENT_TEAM_THREADS);

    let closed = runtime
        .execute_agent_control_tool(
            &first_child,
            &"close-turn".to_string(),
            &ToolCallCompleted {
                id: "close-from-child".to_string(),
                name: "close_agent".to_string(),
                arguments: serde_json::json!({ "target": &second_child }).to_string(),
            },
            serde_json::json!({ "target": &second_child }),
        )
        .await;
    assert!(!closed.is_error, "child close failed: {}", closed.text);
    let replacement = runtime
        .spawn_team_member_for_caller(
            &third_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Replacement after close".to_string(),
                model_provider: None,
                model: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(replacement.id, third.id);
    assert_eq!(
        replacement
            .members
            .iter()
            .filter(|member| {
                member.role == roder_api::teams::TeamMemberRole::Lead
                    || member.status != TeamMemberStatus::Closed
            })
            .count(),
        CODEX_V2_MAX_RESIDENT_TEAM_THREADS
    );

    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

async fn codex_v2_team_runtime(test_name: &str) -> (Arc<Runtime>, ThreadId, PathBuf, PathBuf) {
    let suffix = uuid::Uuid::new_v4();
    let thread_root =
        std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-threads-{suffix}"));
    let team_root = std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-teams-{suffix}"));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                team_data_dir: Some(team_root.clone()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let parent = runtime
        .create_thread_with(CreateThreadRequest {
            title: Some("Codex V2 parent".to_string()),
            workspace: std::env::current_dir().unwrap().display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: Some(roder_api::catalog::PROVIDER_CODEX.to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            selection_mode: Some(ModelSelectionMode::manual(
                roder_api::catalog::PROVIDER_CODEX,
                "gpt-5.6-sol",
                Some(REASONING_ULTRA.to_string()),
            )),
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap();
    (runtime, parent.thread_id, thread_root, team_root)
}
