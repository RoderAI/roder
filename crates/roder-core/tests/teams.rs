use std::sync::Arc;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::*;
use roder_api::policy_mode::PolicyMode;
use roder_api::teams::{AgentTeamDisplayMode, TeamMemberStatus};
use roder_core::{Runtime, RuntimeConfig, TeamMemberStartRequest, TeamStartRequest};

struct PendingEngine;

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
    assert_eq!(
        updated.members[1].current_turn_id.as_deref(),
        Some(turn_a.as_str())
    );
    assert_eq!(
        updated.members[2].current_turn_id.as_deref(),
        Some(turn_b.as_str())
    );
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

fn team_data_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("roder-core-{name}-{}", uuid::Uuid::new_v4()))
}
