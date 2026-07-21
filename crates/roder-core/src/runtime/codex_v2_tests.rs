use super::codex_v2::{CODEX_V2_MAX_AGENT_DEPTH, CODEX_V2_MAX_RESIDENT_TEAM_THREADS};
use super::*;

use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{InferenceCapabilities, InferenceEventStream, InferenceProviderContext};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

struct PendingCodexV2Engine {
    started: tokio::sync::mpsc::UnboundedSender<ThreadId>,
}

#[async_trait::async_trait]
impl InferenceEngine for PendingCodexV2Engine {
    fn id(&self) -> String {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
        Ok(roder_api::catalog::models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let _ = self.started.send(ctx.thread_id.to_string());
        Ok(Box::pin(futures::stream::pending()))
    }
}

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
    runtime
        .persist_turn_item(
            &parent_thread_id,
            &"historic-turn".to_string(),
            &TranscriptItem::UserMessage(UserMessage::text("inherited parent context")),
        )
        .await
        .unwrap();

    let team = runtime
        .spawn_team_member_for_caller(
            &parent_thread_id,
            &parent_turn_id,
            TeamMemberStartRequest {
                name: "Inherited child".to_string(),
                model_provider: None,
                model: None,
            },
            None,
            "all".to_string(),
            None,
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
    assert_eq!(metadata.workspace_id.as_deref(), Some("workspace-test"));
    assert_eq!(metadata.root_id.as_deref(), Some("root-test"));
    assert_eq!(metadata.tool_allowlist, vec!["read".to_string()]);
    assert!(
        metadata
            .developer_instructions
            .as_deref()
            .is_some_and(|instructions| {
                instructions.contains("parent developer contract")
                    && instructions.contains("long-lived collaboration subagent")
            })
    );
    assert_eq!(metadata.parent_thread_id.as_ref(), Some(&parent_thread_id));
    assert_eq!(child.parent_thread_id.as_ref(), Some(&parent_thread_id));
    assert_eq!(child.agent_path.as_deref(), Some("/root/Inherited child"));
    let child_snapshot = runtime
        .load_thread(&child.thread_id)
        .await
        .unwrap()
        .expect("child snapshot");
    assert!(child_snapshot.events.iter().any(|envelope| {
        matches!(
            &envelope.event,
            RoderEvent::TranscriptItemAppended(event)
                if matches!(
                    event.item.as_ref(),
                    Some(TranscriptItem::UserMessage(message))
                        if message.text == "inherited parent context"
                )
        )
    }));

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
            None,
            "all".to_string(),
            None,
        )
        .await
        .unwrap();
    let first_child = first.members.last().unwrap().thread_id.clone();
    let first_member_id = first.members.last().unwrap().id.clone();
    runtime
        .teams
        .update_member(&first.id, &first_member_id, |member| {
            member.status = TeamMemberStatus::Running;
            member.current_turn_id = Some("recursive-first-turn".to_string());
        })
        .await
        .unwrap();
    register_test_active_turn(&runtime, &first_child, "recursive-first-turn").await;
    let second = runtime
        .spawn_team_member_for_caller(
            &first_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Recursive second".to_string(),
                model_provider: None,
                model: None,
            },
            None,
            "all".to_string(),
            None,
        )
        .await
        .unwrap();
    let second_child = second.members.last().unwrap().thread_id.clone();
    let second_member_id = second.members.last().unwrap().id.clone();
    runtime
        .teams
        .update_member(&second.id, &second_member_id, |member| {
            member.status = TeamMemberStatus::Running;
            member.current_turn_id = Some("recursive-second-turn".to_string());
        })
        .await
        .unwrap();
    register_test_active_turn(&runtime, &second_child, "recursive-second-turn").await;
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
            None,
            "all".to_string(),
            None,
        )
        .await
        .unwrap();
    let third_child = third.members.last().unwrap().thread_id.clone();
    let third_member_id = third.members.last().unwrap().id.clone();
    runtime
        .teams
        .update_member(&third.id, &third_member_id, |member| {
            member.status = TeamMemberStatus::Running;
            member.current_turn_id = Some("recursive-third-turn".to_string());
        })
        .await
        .unwrap();
    register_test_active_turn(&runtime, &third_child, "recursive-third-turn").await;

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
            None,
            "all".to_string(),
            None,
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

    let interrupted = runtime
        .execute_agent_control_tool(
            &first_child,
            &"interrupt-turn".to_string(),
            &ToolCallCompleted {
                id: "interrupt-from-child".to_string(),
                name: "interrupt_agent".to_string(),
                arguments: serde_json::json!({ "target": &second_child }).to_string(),
            },
            serde_json::json!({ "target": &second_child }),
        )
        .await;
    assert!(
        !interrupted.is_error,
        "child interrupt failed: {}",
        interrupted.text
    );
    let replacement = runtime
        .spawn_team_member_for_caller(
            &third_child,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "Replacement after close".to_string(),
                model_provider: None,
                model: None,
            },
            None,
            "all".to_string(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(replacement.id, third.id);
    let replacement_member_id = replacement.members.last().unwrap().id.clone();
    runtime
        .teams
        .update_member(&replacement.id, &replacement_member_id, |member| {
            member.status = TeamMemberStatus::Running;
            member.current_turn_id = Some("recursive-replacement-turn".to_string());
        })
        .await
        .unwrap();
    let replacement_thread_id = replacement.members.last().unwrap().thread_id.clone();
    register_test_active_turn(
        &runtime,
        &replacement_thread_id,
        "recursive-replacement-turn",
    )
    .await;
    let replacement = runtime.read_team(&replacement.id).await.unwrap();
    assert_eq!(
        replacement.members.len(),
        CODEX_V2_MAX_RESIDENT_TEAM_THREADS + 1
    );
    assert_eq!(
        1 + replacement
            .members
            .iter()
            .filter(|member| {
                member.role != roder_api::teams::TeamMemberRole::Lead
                    && member.status == TeamMemberStatus::Running
            })
            .count(),
        CODEX_V2_MAX_RESIDENT_TEAM_THREADS
    );

    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_rejects_nesting_beyond_five_before_creating_state() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        codex_v2_team_runtime("nesting-depth").await;
    let test_turn_id = "nesting-depth-turn".to_string();
    let mut caller_thread_id = parent_thread_id;
    let mut expected_path = "/root".to_string();
    let mut team_id = None;

    for depth in 1..=CODEX_V2_MAX_AGENT_DEPTH {
        let task_name = format!("level_{depth}");
        let team = runtime
            .spawn_team_member_for_caller(
                &caller_thread_id,
                &test_turn_id,
                TeamMemberStartRequest {
                    name: task_name.clone(),
                    model_provider: None,
                    model: None,
                },
                None,
                "none".to_string(),
                None,
            )
            .await
            .unwrap();
        expected_path.push('/');
        expected_path.push_str(&task_name);
        let child = team.members.last().unwrap();
        assert_eq!(child.agent_path.as_deref(), Some(expected_path.as_str()));
        caller_thread_id = child.thread_id.clone();
        team_id = Some(team.id);
    }

    let team_id = team_id.unwrap();
    let members_before = runtime.read_team(&team_id).await.unwrap().members.len();
    let threads_before = runtime.list_threads().await.unwrap().len();
    let error = runtime
        .spawn_team_member_for_caller(
            &caller_thread_id,
            &test_turn_id,
            TeamMemberStartRequest {
                name: "level_6".to_string(),
                model_provider: None,
                model: None,
            },
            None,
            "none".to_string(),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("agent nesting depth limit reached: maximum 5 levels below /root"),
        "unexpected nesting error: {error}"
    );
    assert_eq!(
        runtime.read_team(&team_id).await.unwrap().members.len(),
        members_before,
        "rejected nesting must not append a team member"
    );
    assert_eq!(
        runtime.list_threads().await.unwrap().len(),
        threads_before,
        "rejected nesting must not create an orphan thread"
    );

    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_concurrent_spawns_reserve_capacity_through_initial_turn_start() {
    let (runtime, parent_thread_id, thread_root, team_root, _started) =
        codex_v2_pending_runtime("atomic-spawn-capacity").await;
    let parent_turn_id = start_pending_parent_turn(&runtime, &parent_thread_id, None).await;

    let results = futures::future::join_all((0..4).map(|index| {
        let runtime = Arc::clone(&runtime);
        let parent_thread_id = parent_thread_id.clone();
        let parent_turn_id = parent_turn_id.clone();
        async move {
            let task_name = format!("child_{index}");
            runtime
                .execute_agent_control_tool(
                    &parent_thread_id,
                    &parent_turn_id,
                    &control_call(&format!("spawn-{index}"), "spawn_agent"),
                    serde_json::json!({
                        "task_name": task_name,
                        "message": "remain pending",
                        "fork_turns": "none"
                    }),
                )
                .await
        }
    }))
    .await;
    assert_eq!(results.iter().filter(|result| !result.is_error).count(), 3);
    assert_eq!(results.iter().filter(|result| result.is_error).count(), 1);
    assert!(
        results
            .iter()
            .filter(|result| result.is_error)
            .all(|result| {
                result.text.contains("agent thread limit reached")
                    || result
                        .data
                        .to_string()
                        .contains("agent thread limit reached")
            })
    );

    let team = runtime
        .list_teams()
        .await
        .into_iter()
        .find(|team| team.lead_thread_id == parent_thread_id)
        .expect("implicit V2 team");
    let lead = team
        .members
        .iter()
        .find(|member| member.role == roder_api::teams::TeamMemberRole::Lead)
        .unwrap();
    assert_eq!(lead.status, TeamMemberStatus::Running);
    assert_eq!(lead.current_turn_id.as_ref(), Some(&parent_turn_id));
    assert_eq!(
        team.members
            .iter()
            .filter(|member| member.role != roder_api::teams::TeamMemberRole::Lead)
            .count(),
        3
    );
    assert_eq!(
        team.members
            .iter()
            .filter(|member| {
                member.role != roder_api::teams::TeamMemberRole::Lead
                    && member.status == TeamMemberStatus::Running
                    && member.current_turn_id.is_some()
            })
            .count(),
        3
    );

    interrupt_team_turns(&runtime, &team).await;
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_mailbox_bursts_reserve_each_message_once_per_turn() {
    let (runtime, parent_thread_id, thread_root, team_root, mut started) =
        codex_v2_pending_runtime("mailbox-reservations").await;
    let parent_turn_id = start_pending_parent_turn(&runtime, &parent_thread_id, None).await;
    wait_for_inference_start(&mut started, &parent_thread_id).await;
    let spawned = runtime
        .execute_agent_control_tool(
            &parent_thread_id,
            &parent_turn_id,
            &control_call("spawn-mailbox", "spawn_agent"),
            serde_json::json!({
                "task_name": "mailbox_child",
                "message": "remain pending",
                "fork_turns": "none"
            }),
        )
        .await;
    assert!(!spawned.is_error, "{}", spawned.text);
    let child_thread_id = spawned.data["thread_id"].as_str().unwrap().to_string();
    let child_turn_id = spawned.data["turn_id"].as_str().unwrap().to_string();
    wait_for_inference_start(&mut started, &child_thread_id).await;
    let team_id = spawned.data["team_id"].as_str().unwrap().to_string();
    let member_id = spawned.data["member_id"].as_str().unwrap().to_string();

    runtime
        .queue_team_member_message(
            &parent_thread_id,
            &team_id,
            &member_id,
            "first burst message".to_string(),
        )
        .await
        .unwrap();
    runtime
        .queue_team_member_message(
            &parent_thread_id,
            &team_id,
            &member_id,
            "second burst message".to_string(),
        )
        .await
        .unwrap();

    let active = runtime
        .active_turns
        .read()
        .await
        .get(&child_turn_id)
        .cloned()
        .expect("child active turn");
    let steers = active.steers.lock().await.clone();
    assert_eq!(steers.len(), 2);
    assert_eq!(
        steers[0].message.text,
        "Message Type: MESSAGE\nTask name: /root/mailbox_child\nSender: /root\nPayload:\nfirst burst message"
    );
    assert_eq!(
        steers[1].message.text,
        "Message Type: MESSAGE\nTask name: /root/mailbox_child\nSender: /root\nPayload:\nsecond burst message"
    );
    let team = runtime.read_team(&team_id).await.unwrap();
    assert!(
        team.mailbox
            .iter()
            .filter(|message| message.text.contains("burst message"))
            .all(|message| !message.delivered),
        "queued steers are acknowledged only after transcript persistence"
    );

    interrupt_team_turns(&runtime, &team).await;
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_interrupt_followup_reclaims_reserved_mailbox_messages() {
    let (runtime, parent_thread_id, thread_root, team_root, mut started) =
        codex_v2_pending_runtime("interrupt-mailbox-handoff").await;
    let parent_turn_id = start_pending_parent_turn(&runtime, &parent_thread_id, None).await;
    wait_for_inference_start(&mut started, &parent_thread_id).await;
    let spawned = runtime
        .execute_agent_control_tool(
            &parent_thread_id,
            &parent_turn_id,
            &control_call("spawn-interrupt-mailbox", "spawn_agent"),
            serde_json::json!({
                "task_name": "interrupt_child",
                "message": "remain pending",
                "fork_turns": "none"
            }),
        )
        .await;
    assert!(!spawned.is_error, "{}", spawned.text);
    let child_thread_id = spawned.data["thread_id"].as_str().unwrap().to_string();
    let original_turn_id = spawned.data["turn_id"].as_str().unwrap().to_string();
    let team_id = spawned.data["team_id"].as_str().unwrap().to_string();
    let member_id = spawned.data["member_id"].as_str().unwrap().to_string();
    wait_for_inference_start(&mut started, &child_thread_id).await;

    runtime
        .queue_team_member_message(
            &parent_thread_id,
            &team_id,
            &member_id,
            "survive interrupt".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .interrupt_team_member(&team_id, &member_id)
            .await
            .unwrap()
            .as_deref(),
        Some(original_turn_id.as_str())
    );
    let replacement_turn_id = runtime
        .followup_team_member(
            &parent_thread_id,
            &team_id,
            &member_id,
            "replacement assignment".to_string(),
        )
        .await
        .unwrap();
    assert_ne!(replacement_turn_id, original_turn_id);
    wait_for_inference_start(&mut started, &child_thread_id).await;

    let snapshot = runtime
        .load_thread(&child_thread_id)
        .await
        .unwrap()
        .expect("replacement child snapshot");
    let replacement_input = snapshot
        .turns
        .iter()
        .find(|turn| turn.turn_id == replacement_turn_id)
        .and_then(|turn| {
            turn.items.iter().find_map(|item| match item {
                TranscriptItem::UserMessage(message) => Some(message.text.as_str()),
                _ => None,
            })
        })
        .expect("replacement input");
    assert_eq!(
        replacement_input,
        "Message Type: MESSAGE\nTask name: /root/interrupt_child\nSender: /root\nPayload:\nsurvive interrupt\n\nMessage Type: NEW_TASK\nTask name: /root/interrupt_child\nSender: /root\nPayload:\nreplacement assignment"
    );
    let team = runtime.read_team(&team_id).await.unwrap();
    for text in ["survive interrupt", "replacement assignment"] {
        assert!(
            team.mailbox
                .iter()
                .any(|message| message.text == text && message.delivered),
            "replacement turn must acknowledge {text:?}"
        );
    }

    interrupt_team_turns(&runtime, &team).await;
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_full_history_child_treats_new_task_as_authoritative_assignment() {
    let (runtime, parent_thread_id, thread_root, team_root, mut started) =
        codex_v2_pending_runtime("typed-new-task-envelope").await;
    runtime
        .persist_turn_item(
            &parent_thread_id,
            &"historic-parent-orchestration".to_string(),
            &TranscriptItem::UserMessage(UserMessage::text(
                "Spawn final_probe and orchestrate the workers.",
            )),
        )
        .await
        .unwrap();
    let parent_turn_id = start_pending_parent_turn(&runtime, &parent_thread_id, None).await;
    wait_for_inference_start(&mut started, &parent_thread_id).await;

    let spawned = runtime
        .execute_agent_control_tool(
            &parent_thread_id,
            &parent_turn_id,
            &control_call("spawn-typed-child", "spawn_agent"),
            serde_json::json!({
                "task_name": "typed_child",
                "message": "Only audit typed envelope rendering; do not spawn subagents.",
                "agent_type": "release-audit",
                "fork_turns": "all"
            }),
        )
        .await;
    assert!(!spawned.is_error, "{}", spawned.text);
    let child_thread_id = spawned.data["thread_id"].as_str().unwrap().to_string();
    wait_for_inference_start(&mut started, &child_thread_id).await;

    let snapshot = runtime
        .load_thread(&child_thread_id)
        .await
        .unwrap()
        .expect("child snapshot");
    let user_messages = snapshot
        .events
        .iter()
        .filter_map(|envelope| match &envelope.event {
            RoderEvent::TranscriptItemAppended(event) => match event.item.as_ref() {
                Some(TranscriptItem::UserMessage(message)) => Some(message.text.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        user_messages.contains(&"Spawn final_probe and orchestrate the workers."),
        "forked parent request must remain available as history: {user_messages:?}"
    );
    assert_eq!(
        user_messages.last().copied(),
        Some(
            "Message Type: NEW_TASK\nTask name: /root/typed_child\nSender: /root\nPayload:\nOnly audit typed envelope rendering; do not spawn subagents."
        ),
        "the newest child input must be the typed authoritative assignment"
    );
    let metadata = runtime
        .load_thread_metadata(&child_thread_id)
        .await
        .unwrap()
        .expect("child metadata");
    let developer = metadata
        .developer_instructions
        .expect("child identity instructions");
    assert!(developer.contains("Forked conversation history is context only."));
    assert!(developer.contains("`Message Type: NEW_TASK` is the authoritative current assignment"));
    assert!(developer.contains("Do not repeat or continue parent orchestration"));
    assert!(developer.contains("Your assigned agent type is \"release-audit\"."));

    let team = runtime
        .read_team(spawned.data["team_id"].as_str().unwrap())
        .await
        .unwrap();
    interrupt_team_turns(&runtime, &team).await;
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}

#[tokio::test]
async fn codex_v2_nested_children_inherit_live_authority_with_one_direct_parent_identity() {
    let (runtime, parent_thread_id, thread_root, team_root, mut started) =
        codex_v2_pending_runtime("live-developer-authority").await;
    let parent_turn_id = start_pending_parent_turn(
        &runtime,
        &parent_thread_id,
        Some(("live bundle authority", "host turn authority")),
    )
    .await;
    wait_for_inference_start(&mut started, &parent_thread_id).await;
    let child = runtime
        .execute_agent_control_tool(
            &parent_thread_id,
            &parent_turn_id,
            &control_call("spawn-child", "spawn_agent"),
            serde_json::json!({
                "task_name": "child",
                "message": "remain pending",
                "fork_turns": "none"
            }),
        )
        .await;
    assert!(!child.is_error, "{}", child.text);
    let child_thread_id = child.data["thread_id"].as_str().unwrap().to_string();
    let child_turn_id = child.data["turn_id"].as_str().unwrap().to_string();
    wait_for_inference_start(&mut started, &child_thread_id).await;
    let child_context = runtime
        .active_turn_contexts
        .read()
        .await
        .get(&child_turn_id)
        .cloned()
        .expect("child live context");
    assert_eq!(
        child_context.instructions.developer.as_deref(),
        Some("live bundle authority")
    );
    assert_eq!(
        child_context.developer_context.as_deref(),
        Some("host turn authority")
    );

    let grandchild = runtime
        .execute_agent_control_tool(
            &child_thread_id,
            &child_turn_id,
            &control_call("spawn-grandchild", "spawn_agent"),
            serde_json::json!({
                "task_name": "grandchild",
                "message": "remain pending",
                "agent_type": "reviewer",
                "fork_turns": "none"
            }),
        )
        .await;
    assert!(!grandchild.is_error, "{}", grandchild.text);
    let grandchild_thread_id = grandchild.data["thread_id"].as_str().unwrap();
    let grandchild_turn_id = grandchild.data["turn_id"].as_str().unwrap();
    wait_for_inference_start(&mut started, &grandchild_thread_id.to_string()).await;
    let grandchild_metadata = runtime
        .load_thread_metadata(&grandchild_thread_id.to_string())
        .await
        .unwrap()
        .expect("grandchild metadata");
    let developer = grandchild_metadata
        .developer_instructions
        .expect("grandchild developer instructions");
    assert!(developer.contains("parent developer contract"));
    assert!(developer.contains("at /root/child/grandchild"));
    assert!(developer.contains("direct parent is /root/child"));
    assert!(developer.contains("findings to /root/child"));
    assert_eq!(developer.matches("<roder-subagent-identity>").count(), 1);
    assert!(!developer.contains("at /root/child."));

    runtime
        .queue_team_member_message(
            &child_thread_id,
            grandchild.data["team_id"].as_str().unwrap(),
            grandchild.data["member_id"].as_str().unwrap(),
            "nested coordination".to_string(),
        )
        .await
        .unwrap();
    let active = runtime
        .active_turns
        .read()
        .await
        .get(grandchild_turn_id)
        .cloned()
        .expect("grandchild active turn");
    let steers = active.steers.lock().await.clone();
    assert_eq!(steers.len(), 1);
    assert_eq!(
        steers[0].message.text,
        "Message Type: MESSAGE\nTask name: /root/child/grandchild\nSender: /root/child\nPayload:\nnested coordination"
    );

    let team_id = child.data["team_id"].as_str().unwrap();
    let child_member_id = child.data["member_id"].as_str().unwrap();
    assert_eq!(
        runtime
            .interrupt_team_member(team_id, child_member_id)
            .await
            .unwrap()
            .as_deref(),
        Some(child_turn_id.as_str())
    );
    let resumed_child_turn_id = runtime
        .followup_team_member(
            &parent_thread_id,
            team_id,
            child_member_id,
            "second assignment".to_string(),
        )
        .await
        .unwrap();
    assert_ne!(resumed_child_turn_id, child_turn_id);
    wait_for_inference_start(&mut started, &child_thread_id).await;
    let resumed_context = runtime
        .active_turn_contexts
        .read()
        .await
        .get(&resumed_child_turn_id)
        .cloned()
        .expect("resumed child live context");
    assert_eq!(
        resumed_context.instructions.developer.as_deref(),
        Some("live bundle authority")
    );
    assert_eq!(
        resumed_context.developer_context.as_deref(),
        Some("host turn authority")
    );
    let resumed_selection = runtime
        .active_turn_selections
        .read()
        .await
        .get(&resumed_child_turn_id)
        .cloned();
    assert_eq!(
        resumed_selection,
        Some(ModelSelectionMode::manual(
            PROVIDER_MOCK,
            "gpt-5.6-sol",
            Some(REASONING_ULTRA.to_string()),
        ))
    );

    let team = runtime
        .read_team(child.data["team_id"].as_str().unwrap())
        .await
        .unwrap();
    assert!(
        runtime
            .team_member_turn_contexts
            .lock()
            .await
            .contains_key(&child_thread_id),
        "reusable child authority must remain retained between turns"
    );
    interrupt_team_turns(&runtime, &team).await;
    assert!(runtime.cleanup_team(&team.id, true).await.unwrap());
    let retained_contexts = runtime.team_member_turn_contexts.lock().await;
    assert!(!retained_contexts.contains_key(&child_thread_id));
    assert!(!retained_contexts.contains_key(grandchild_thread_id));
    drop(retained_contexts);
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
            workspace_id: Some("workspace-test".to_string()),
            root_id: Some("root-test".to_string()),
            provider: Some(roder_api::catalog::PROVIDER_CODEX.to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            selection_mode: Some(ModelSelectionMode::manual(
                roder_api::catalog::PROVIDER_CODEX,
                "gpt-5.6-sol",
                Some(REASONING_ULTRA.to_string()),
            )),
            tool_allowlist: vec!["read".to_string()],
            developer_instructions: Some("parent developer contract".to_string()),
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap();
    (runtime, parent.thread_id, thread_root, team_root)
}

async fn codex_v2_pending_runtime(
    test_name: &str,
) -> (
    Arc<Runtime>,
    ThreadId,
    PathBuf,
    PathBuf,
    tokio::sync::mpsc::UnboundedReceiver<ThreadId>,
) {
    let suffix = uuid::Uuid::new_v4();
    let thread_root =
        std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-threads-{suffix}"));
    let team_root = std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-teams-{suffix}"));
    let (started_tx, started_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingCodexV2Engine {
        started: started_tx,
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "gpt-5.6-sol".to_string(),
                reasoning: Some(REASONING_ULTRA.to_string()),
                team_data_dir: Some(team_root.clone()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let parent = runtime
        .create_thread_with(CreateThreadRequest {
            title: Some("Codex V2 pending parent".to_string()),
            workspace: std::env::current_dir().unwrap().display().to_string(),
            workspace_id: Some("workspace-test".to_string()),
            root_id: Some("root-test".to_string()),
            provider: Some(PROVIDER_MOCK.to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            selection_mode: Some(ModelSelectionMode::manual(
                PROVIDER_MOCK,
                "gpt-5.6-sol",
                Some(REASONING_ULTRA.to_string()),
            )),
            tool_allowlist: Vec::new(),
            developer_instructions: Some("parent developer contract".to_string()),
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap();
    (
        runtime,
        parent.thread_id,
        thread_root,
        team_root,
        started_rx,
    )
}

async fn start_pending_parent_turn(
    runtime: &Arc<Runtime>,
    parent_thread_id: &ThreadId,
    authority: Option<(&str, &str)>,
) -> TurnId {
    runtime
        .start_turn(StartTurnRequest {
            thread_id: parent_thread_id.clone(),
            message: "root work".to_string(),
            images: Vec::new(),
            provider_override: Some(PROVIDER_MOCK.to_string()),
            model_override: Some("gpt-5.6-sol".to_string()),
            reasoning_override: Some(REASONING_ULTRA.to_string()),
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: InstructionBundle {
                system: Some("test system".to_string()),
                developer: authority.map(|(developer, _)| developer.to_string()),
                developer_context: None,
            },
            developer_context: authority.map(|(_, context)| context.to_string()),
            task_ledger_required: false,
        })
        .await
        .unwrap()
}

fn control_call(id: &str, name: &str) -> ToolCallCompleted {
    ToolCallCompleted {
        id: id.to_string(),
        name: name.to_string(),
        arguments: "{}".to_string(),
    }
}

async fn wait_for_inference_start(
    started: &mut tokio::sync::mpsc::UnboundedReceiver<ThreadId>,
    expected_thread_id: &ThreadId,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if started.recv().await.as_ref() == Some(expected_thread_id) {
                break;
            }
        }
    })
    .await
    .expect("turn must reach pending inference");
}

async fn interrupt_team_turns(runtime: &Arc<Runtime>, team: &TeamState) {
    for member in team.members.iter().rev() {
        if let Some(turn_id) = member.current_turn_id.clone() {
            runtime
                .interrupt_turn(member.thread_id.clone(), turn_id)
                .await
                .unwrap();
        }
    }
}

async fn register_test_active_turn(runtime: &Arc<Runtime>, thread_id: &ThreadId, turn_id: &str) {
    let (abort, _registration) = AbortHandle::new_pair();
    runtime.active_turns.write().await.insert(
        turn_id.to_string(),
        ActiveTurnHandle {
            thread_id: thread_id.clone(),
            abort,
            steers: Arc::new(Mutex::new(Vec::new())),
            drain: Arc::new(TurnDrainHandle {
                thread_id: thread_id.clone(),
                interrupt_requested: AtomicBool::new(false),
                interrupt_reason: Mutex::new(None),
                completed: AtomicBool::new(false),
                completed_notify: Notify::new(),
            }),
        },
    );
}

#[tokio::test]
async fn runtime_drain_marks_stuck_turn_as_timed_out() {
    let (runtime, thread_id, thread_root, team_root) = codex_v2_team_runtime("drain-timeout").await;
    let turn_id = "drain-timeout-turn".to_string();
    register_test_active_turn(&runtime, &thread_id, &turn_id).await;

    let outcome = runtime
        .drain_active_turns(std::time::Duration::from_millis(1))
        .await;
    assert_eq!(
        outcome,
        RuntimeDrainOutcome::DeadlineExceeded {
            interrupted_turn_ids: vec![turn_id.clone()],
            remaining_turn_ids: vec![turn_id.clone()],
        }
    );

    let lifecycle = runtime.turn_lifecycle_snapshot(&thread_id).await.unwrap();
    assert!(lifecycle.records.iter().any(|record| {
        record.turn_id == turn_id
            && record.state == TurnLifecycleState::InterruptRequested
            && record.cleanup == TurnCleanupState::TimedOut
            && record.reason == Some(TurnLifecycleReason::Shutdown)
    }));
    let metrics = runtime.lifecycle_metrics();
    assert_eq!(metrics.shutdown_drain_count, 1);
    assert_eq!(metrics.deadline_exceeded_count, 1);
    assert_eq!(metrics.clean_shutdown_count, 0);
    assert_eq!(metrics.persistence_failed_count, 0);
    assert!(
        metrics.shutdown_drain_duration_ms_total >= 1,
        "deadline fixture must contribute a bounded drain duration"
    );

    runtime.turn_drains.write().await.remove(&turn_id);
    runtime.resume_accepting_turns();
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}
