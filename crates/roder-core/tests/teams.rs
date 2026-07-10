use std::sync::Arc;

use futures::{future::join_all, stream};
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::*;
use roder_api::policy_mode::PolicyMode;
use roder_api::teams::{
    AgentTeamDisplayMode, TeamMailboxMessageKind, TeamMemberDescriptor, TeamMemberRole,
    TeamMemberStatus,
};
use roder_core::{
    Runtime, RuntimeConfig, TeamManager, TeamMemberStartRequest, TeamStartRequest, TeamState,
};
use time::OffsetDateTime;

struct PendingEngine;

struct StartupFailureEngine;

#[async_trait::async_trait]
impl InferenceEngine for PendingEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(stream::pending()))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for StartupFailureEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        anyhow::bail!("startup exploded")
    }
}

#[tokio::test]
async fn team_runtime_starts_members_as_independent_threads() {
    let runtime = runtime_with_fake_engine("team_runtime_starts_members_as_independent_threads");

    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![
                TeamMemberStartRequest {
                    name: "Reviewer".to_string(),
                    model_provider: None,
                    model: None,
                },
                TeamMemberStartRequest {
                    name: "Builder".to_string(),
                    model_provider: None,
                    model: None,
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(team.members.len(), 3);
    assert_ne!(team.members[1].thread_id, team.members[2].thread_id);

    let turn_a = runtime
        .message_team_member(&team.id, &team.members[1].id, "inspect".to_string())
        .await
        .unwrap();
    let turn_b = runtime
        .message_team_member(&team.id, &team.members[2].id, "build".to_string())
        .await
        .unwrap();

    assert_ne!(turn_a, turn_b);
    let updated = runtime.read_team(&team.id).await.unwrap();
    for (member, turn_id) in [
        (&updated.members[1], &turn_a),
        (&updated.members[2], &turn_b),
    ] {
        assert!(
            (member.current_turn_id.as_deref() == Some(turn_id.as_str())
                && member.status == TeamMemberStatus::Running)
                || (member.current_turn_id.is_none()
                    && member.status == TeamMemberStatus::Completed),
            "a fast completion must remain completed instead of being overwritten as running: {member:?}"
        );
    }
    assert_eq!(updated.mailbox.len(), 2);
    assert_eq!(updated.mailbox[0].to_member_id, updated.members[1].id);
    assert_eq!(updated.mailbox[0].text, "inspect");
}

#[tokio::test]
async fn team_cleanup_refuses_running_members_unless_forced() {
    let runtime = runtime_with_pending_engine("team_cleanup_refuses_running_members_unless_forced");
    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![TeamMemberStartRequest {
                name: "Long runner".to_string(),
                model_provider: None,
                model: None,
            }],
        })
        .await
        .unwrap();

    let first_turn = runtime
        .message_team_member(&team.id, &team.members[1].id, "wait".to_string())
        .await
        .unwrap();
    let steered_turn = runtime
        .message_team_member(&team.id, &team.members[1].id, "keep waiting".to_string())
        .await
        .unwrap();
    assert_eq!(first_turn, steered_turn);
    let err = runtime.cleanup_team(&team.id, false).await.unwrap_err();
    assert!(err.to_string().contains("active teammates"));

    let interrupted = runtime
        .interrupt_team_member(&team.id, &team.members[1].id)
        .await
        .unwrap();
    assert!(interrupted.is_some());
    let updated = runtime.read_team(&team.id).await.unwrap();
    assert_eq!(updated.members[1].status, TeamMemberStatus::Interrupted);

    assert!(runtime.cleanup_team(&team.id, false).await.unwrap());
    assert!(runtime.read_team(&team.id).await.is_none());
}

#[tokio::test]
async fn team_member_startup_failure_releases_running_slot_and_records_terminal_error() {
    let runtime = runtime_with_startup_failure_engine(
        "team_member_startup_failure_releases_running_slot_and_records_terminal_error",
    );
    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![TeamMemberStartRequest {
                name: "Failure".to_string(),
                model_provider: None,
                model: None,
            }],
        })
        .await
        .unwrap();

    runtime
        .message_team_member(&team.id, &team.members[1].id, "fail".to_string())
        .await
        .unwrap();

    let failed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let member = runtime.read_team(&team.id).await.unwrap().members[1].clone();
            if member.status == TeamMemberStatus::Failed {
                break member;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("startup failure must terminate the member");
    assert_eq!(failed.current_turn_id, None);
    assert!(
        failed
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("startup exploded"))
    );
}

#[tokio::test]
async fn direct_turn_interrupt_reconciles_team_member_terminal_state() {
    let runtime =
        runtime_with_pending_engine("direct_turn_interrupt_reconciles_team_member_terminal_state");
    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![TeamMemberStartRequest {
                name: "Interruptible".to_string(),
                model_provider: None,
                model: None,
            }],
        })
        .await
        .unwrap();
    let member = team.members[1].clone();
    let turn_id = runtime
        .message_team_member(&team.id, &member.id, "wait".to_string())
        .await
        .unwrap();

    runtime
        .interrupt_turn(member.thread_id, turn_id)
        .await
        .unwrap();
    let interrupted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let member = runtime.read_team(&team.id).await.unwrap().members[1].clone();
            if member.status == TeamMemberStatus::Interrupted {
                break member;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("direct turn interruption must terminate the team member");
    assert_eq!(interrupted.current_turn_id, None);
}

#[tokio::test]
async fn interrupt_and_followup_never_orphan_a_replacement_turn() {
    let runtime =
        runtime_with_pending_engine("interrupt_and_followup_never_orphan_a_replacement_turn");
    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![TeamMemberStartRequest {
                name: "Racy".to_string(),
                model_provider: None,
                model: None,
            }],
        })
        .await
        .unwrap();
    let member = team.members[1].clone();
    let original_turn_id = runtime
        .message_team_member(&team.id, &member.id, "first".to_string())
        .await
        .unwrap();

    let interrupt_runtime = Arc::clone(&runtime);
    let interrupt_team_id = team.id.clone();
    let interrupt_member_id = member.id.clone();
    let followup_runtime = Arc::clone(&runtime);
    let followup_team_id = team.id.clone();
    let followup_member_id = member.id.clone();
    let (interrupted, followup) = tokio::join!(
        async move {
            interrupt_runtime
                .interrupt_team_member(&interrupt_team_id, &interrupt_member_id)
                .await
        },
        async move {
            followup_runtime
                .message_team_member(
                    &followup_team_id,
                    &followup_member_id,
                    "replacement".to_string(),
                )
                .await
        }
    );
    interrupted.unwrap();
    let followup_turn_id = followup.unwrap();
    let updated = runtime.read_team(&team.id).await.unwrap().members[1].clone();
    let active_turn_id = runtime.active_turn_for_thread(&updated.thread_id).await;
    match updated.status {
        TeamMemberStatus::Running => {
            assert_ne!(followup_turn_id, original_turn_id);
            assert_eq!(updated.current_turn_id, active_turn_id);
            assert_eq!(updated.current_turn_id.as_ref(), Some(&followup_turn_id));
            runtime
                .interrupt_team_member(&team.id, &updated.id)
                .await
                .unwrap();
        }
        TeamMemberStatus::Interrupted => {
            assert_eq!(followup_turn_id, original_turn_id);
            assert_eq!(updated.current_turn_id, None);
            assert_eq!(active_turn_id, None);
        }
        status => panic!("unexpected terminal race outcome: {status:?}"),
    }
}

#[tokio::test]
async fn team_policy_mode_changes_only_selected_member() {
    let runtime = runtime_with_pending_engine("team_policy_mode_changes_only_selected_member");
    let team = runtime
        .start_team(TeamStartRequest {
            lead_thread_id: None,
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![
                TeamMemberStartRequest {
                    name: "Strict".to_string(),
                    model_provider: None,
                    model: None,
                },
                TeamMemberStartRequest {
                    name: "Loose".to_string(),
                    model_provider: None,
                    model: None,
                },
            ],
        })
        .await
        .unwrap();

    runtime
        .set_team_member_policy_mode(&team.id, &team.members[1].id, PolicyMode::Plan)
        .await
        .unwrap();

    let updated = runtime.read_team(&team.id).await.unwrap();
    assert_eq!(updated.members[1].policy_mode, PolicyMode::Plan);
    assert_eq!(updated.members[2].policy_mode, PolicyMode::Default);
    assert_eq!(
        runtime
            .effective_policy_mode_for_thread(&updated.members[1].thread_id)
            .await,
        PolicyMode::Plan
    );
    assert_eq!(
        runtime
            .effective_policy_mode_for_thread(&updated.members[2].thread_id)
            .await,
        PolicyMode::Default
    );
}

#[tokio::test]
async fn team_manager_concurrent_mutations_preserve_completions_and_mailbox_messages() {
    let data_dir = team_data_dir(
        "team_manager_concurrent_mutations_preserve_completions_and_mailbox_messages",
    );
    let manager = Arc::new(TeamManager::new(data_dir.clone()));
    manager
        .insert(manager_test_team("team-atomic"))
        .await
        .unwrap();

    let complete_a = {
        let manager = Arc::clone(&manager);
        async move {
            manager
                .complete_member_turn(
                    "thread-a",
                    "turn-a",
                    TeamMemberStatus::Completed,
                    Some("result a".to_string()),
                    None,
                )
                .await
                .unwrap()
        }
    };
    let complete_b = {
        let manager = Arc::clone(&manager);
        async move {
            manager
                .complete_member_turn(
                    "thread-b",
                    "turn-b",
                    TeamMemberStatus::Failed,
                    None,
                    Some("failure b".to_string()),
                )
                .await
                .unwrap()
        }
    };
    let append_messages = {
        let manager = Arc::clone(&manager);
        async move {
            join_all((0..24).map(|index| {
                let manager = Arc::clone(&manager);
                async move {
                    let member_id = if index % 2 == 0 {
                        "member-a"
                    } else {
                        "member-b"
                    };
                    manager
                        .append_mailbox_message(
                            "team-atomic",
                            Some("lead".to_string()),
                            member_id.to_string(),
                            TeamMailboxMessageKind::Message,
                            format!("message-{index}"),
                        )
                        .await
                        .unwrap();
                }
            }))
            .await;
        }
    };

    let (completed_a, completed_b, ()) = tokio::join!(complete_a, complete_b, append_messages);
    assert_eq!(
        completed_a.unwrap().1.final_message.as_deref(),
        Some("result a")
    );
    assert_eq!(
        completed_b.unwrap().1.terminal_error.as_deref(),
        Some("failure b")
    );

    let updated = manager.get("team-atomic").await.unwrap();
    assert_eq!(updated.mailbox.len(), 24);
    assert_eq!(updated.members[1].status, TeamMemberStatus::Completed);
    assert_eq!(
        updated.members[1].final_message.as_deref(),
        Some("result a")
    );
    assert_eq!(updated.members[2].status, TeamMemberStatus::Failed);
    assert_eq!(
        updated.members[2].terminal_error.as_deref(),
        Some("failure b")
    );

    let delivery_turn = "delivery-turn".to_string();
    let pending = manager
        .reserve_pending_mailbox_messages("team-atomic", "member-a", &delivery_turn)
        .await
        .unwrap();
    assert_eq!(pending.len(), 12);
    assert!(pending.iter().all(|message| !message.delivered));
    assert!(
        manager
            .reserve_pending_mailbox_messages(
                "team-atomic",
                "member-a",
                &"competing-turn".to_string(),
            )
            .await
            .unwrap()
            .is_empty()
    );
    let before_ack = manager.get("team-atomic").await.unwrap();
    assert!(
        before_ack
            .mailbox
            .iter()
            .all(|message| { message.to_member_id != "member-a" || !message.delivered })
    );
    manager
        .mark_mailbox_messages_delivered(
            "team-atomic",
            &delivery_turn,
            &pending
                .iter()
                .map(|message| message.id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    let after_delivery = manager.get("team-atomic").await.unwrap();
    assert!(
        after_delivery
            .mailbox
            .iter()
            .all(|message| { message.to_member_id != "member-a" || message.delivered })
    );
    assert!(
        after_delivery
            .mailbox
            .iter()
            .all(|message| message.to_member_id != "member-b" || !message.delivered)
    );
    let failed_turn = "failed-delivery-turn".to_string();
    assert_eq!(
        manager
            .reserve_pending_mailbox_messages("team-atomic", "member-b", &failed_turn)
            .await
            .unwrap()
            .len(),
        12
    );
    manager
        .release_mailbox_reservations_for_turn(&failed_turn)
        .await;
    let retry_turn = "retry-delivery-turn".to_string();
    let retried = manager
        .reserve_pending_mailbox_messages("team-atomic", "member-b", &retry_turn)
        .await
        .unwrap();
    assert_eq!(
        retried.len(),
        12,
        "failed delivery must remain eligible for retry"
    );
    let retried_ids = retried
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    manager
        .mark_mailbox_messages_delivered("team-atomic", &failed_turn, &retried_ids)
        .await
        .unwrap();
    assert!(
        manager
            .get("team-atomic")
            .await
            .unwrap()
            .mailbox
            .iter()
            .all(|message| message.to_member_id != "member-b" || !message.delivered),
        "a stale turn acknowledgement must not consume a replacement reservation"
    );
    manager
        .mark_mailbox_messages_delivered("team-atomic", &retry_turn, &retried_ids)
        .await
        .unwrap();
    assert!(
        manager
            .get("team-atomic")
            .await
            .unwrap()
            .mailbox
            .iter()
            .all(|message| message.to_member_id != "member-b" || message.delivered)
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn team_manager_lists_persisted_teams_without_mutating_live_process_state() {
    let data_dir =
        team_data_dir("team_manager_lists_persisted_teams_without_mutating_live_process_state");
    let first = TeamManager::new(data_dir.clone());
    first
        .insert(manager_test_team("team-restart"))
        .await
        .unwrap();
    drop(first);

    let second = TeamManager::new(data_dir.clone());
    let listed = second.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "team-restart");
    for member in listed[0]
        .members
        .iter()
        .filter(|member| member.role == TeamMemberRole::Teammate)
    {
        assert_eq!(member.status, TeamMemberStatus::Running);
        assert!(member.current_turn_id.is_some());
    }
    assert_eq!(listed[0].members[1].task_name.as_deref(), Some("task_a"));
    assert_eq!(
        listed[0].members[1].agent_path.as_deref(),
        Some("/root/task_a")
    );
    assert_eq!(
        listed[0].members[1].parent_thread_id.as_deref(),
        Some("thread-lead")
    );
    drop(second);

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Runtime::new(
        builder.build().unwrap(),
        RuntimeConfig {
            team_data_dir: Some(data_dir.clone()),
            ..RuntimeConfig::default()
        },
    )
    .unwrap();
    let local_view = runtime.list_teams().await;
    assert_eq!(
        local_view[0].members[1].status,
        TeamMemberStatus::Interrupted
    );
    assert_eq!(local_view[0].members[1].current_turn_id, None);
    drop(runtime);

    let third = TeamManager::new(data_dir.clone());
    let persisted = third.get("team-restart").await.unwrap();
    assert_eq!(persisted.members[1].status, TeamMemberStatus::Running);
    assert_eq!(
        persisted.members[1].current_turn_id.as_deref(),
        Some("turn-a")
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

fn manager_test_team(id: &str) -> TeamState {
    let now = OffsetDateTime::now_utc();
    TeamState {
        id: id.to_string(),
        lead_thread_id: "thread-lead".to_string(),
        display_mode: AgentTeamDisplayMode::InProcess,
        members: vec![
            manager_test_member(
                "lead",
                TeamMemberRole::Lead,
                "Lead",
                "root",
                "/root",
                "thread-lead",
                None,
                None,
                TeamMemberStatus::Idle,
            ),
            manager_test_member(
                "member-a",
                TeamMemberRole::Teammate,
                "Task A",
                "task_a",
                "/root/task_a",
                "thread-a",
                Some("thread-lead"),
                Some("turn-a"),
                TeamMemberStatus::Running,
            ),
            manager_test_member(
                "member-b",
                TeamMemberRole::Teammate,
                "Task B",
                "task_b",
                "/root/task_b",
                "thread-b",
                Some("thread-lead"),
                Some("turn-b"),
                TeamMemberStatus::Running,
            ),
        ],
        mailbox: Vec::new(),
        tasks: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[allow(clippy::too_many_arguments)]
fn manager_test_member(
    id: &str,
    role: TeamMemberRole,
    name: &str,
    task_name: &str,
    agent_path: &str,
    thread_id: &str,
    parent_thread_id: Option<&str>,
    current_turn_id: Option<&str>,
    status: TeamMemberStatus,
) -> TeamMemberDescriptor {
    TeamMemberDescriptor {
        id: id.to_string(),
        role,
        name: name.to_string(),
        task_name: Some(task_name.to_string()),
        agent_path: Some(agent_path.to_string()),
        thread_id: thread_id.to_string(),
        parent_thread_id: parent_thread_id.map(str::to_string),
        current_turn_id: current_turn_id.map(str::to_string),
        model_provider: Some(PROVIDER_MOCK.to_string()),
        model: Some("mock".to_string()),
        policy_mode: PolicyMode::Default,
        status,
        final_message: None,
        terminal_error: None,
        pane_id: None,
    }
}

fn runtime_with_fake_engine(name: &str) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(roder_core::fake_provider::FakeInferenceEngine));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                team_data_dir: Some(team_data_dir(name)),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn runtime_with_pending_engine(name: &str) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                team_data_dir: Some(team_data_dir(name)),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn runtime_with_startup_failure_engine(name: &str) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(StartupFailureEngine));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                team_data_dir: Some(team_data_dir(name)),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn team_data_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("roder-core-{name}-{}", uuid::Uuid::new_v4()))
}
