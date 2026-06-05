use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use roder_api::catalog::built_in_model_profile;
use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, HostedWebSearchMode, InferenceEngine,
    InferenceEvent, InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ProviderAuthType, ReasoningConfig,
    RuntimeHints, RuntimeProfile,
};
use roder_api::media::{MediaArtifact, MediaAttachment, data_url};
use roder_api::memory::{MemoryProviderSelection, MemoryQuery, MemoryRecord};
use roder_api::plan_review::{HunkRecord, PlanComment, PlanReview, PlanReviewStatus, PlanRewrite};
use roder_api::policy_mode::PolicyDecision;
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext};
use roder_api::transcript::{InputImage, TranscriptItem, UserMessage};
use roder_api::workflow::{
    WorkflowImportDecision, WorkflowImportDecisionKind, WorkflowImportItem, WorkflowImportScan,
    WorkflowImportState,
};
use roder_commands::{
    CommandDirectory, CommandExpansionOptions, CommandExpansionRequest, CommandSource, CommandSpec,
    CommandsRegistry, CommandsRegistryOptions, WorkflowCommandDirectory, expand_command,
};
use roder_core::{
    CreateThreadRequest, Runtime, StartTurnRequest,
    TeamMemberStartRequest as RuntimeTeamMemberStartRequest,
    TeamStartRequest as RuntimeTeamStartRequest, TeamState, default_instructions,
    media_artifacts::{MediaArtifactStore, default_media_artifact_dir},
    policy_gate::DefaultPolicyGate,
};
use roder_protocol::*;
use roder_roadmap::{ListOptions, list_documents, parse_document, validate_document};
use roder_tasks::BackgroundRunner;
use time::OffsetDateTime;
use tokio::sync::{OnceCell, RwLock, broadcast};

use crate::automations::AppServerFeatureConfig;
use crate::notifications;
use crate::protocol_contract::{
    idle_thread_status, protocol_thread_from_metadata, protocol_turn_images, protocol_turn_message,
    protocol_turns_from_snapshot, thread_status_for_activity,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapPathParams {
    path: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapCreateParams {
    slug: String,
    title: String,
    goal: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapPatchParams {
    path: String,
    old_text: String,
    new_text: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapTaskUpdateParams {
    path: String,
    task_id: String,
    checked: bool,
    evidence: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapValidateParams {
    path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoadmapThreadParams {
    path: String,
    task_id: Option<String>,
    thread_id: Option<String>,
    title: Option<String>,
}

pub struct AppServer {
    pub runtime: Arc<Runtime>,
    pub(crate) workflows: crate::workflows::AppWorkflowService,
    pub(crate) tasks: BackgroundRunner,
    pub(crate) persist_user_config: bool,
    pub(crate) features: AppServerFeatureConfig,
    pub(crate) automation_supervisor: Option<roder_automations::AutomationSupervisorHandle>,
    pub(crate) protocol_threads: RwLock<std::collections::HashMap<String, Thread>>,
    pub(crate) protocol_thread_models:
        RwLock<std::collections::HashMap<String, ProtocolThreadModelSelection>>,
    pub(crate) protocol_notifications: broadcast::Sender<JsonRpcNotification>,
    pub(crate) workspaces: crate::workspaces::WorkspaceRegistry,
    pub(crate) workspace_files: crate::workspace_files::WorkspaceFileService,
    pub(crate) command_registry: OnceCell<CommandsRegistry>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProtocolThreadModelSelection {
    pub provider: String,
    pub model: String,
    pub reasoning: Option<String>,
}

impl AppServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self::with_feature_config(runtime, AppServerFeatureConfig::default())
    }

    pub fn with_user_config_persistence(mut self) -> Self {
        self.persist_user_config = true;
        self
    }

    pub(crate) fn persist_user_config_enabled(&self) -> bool {
        self.persist_user_config
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let result = match req.method.as_str() {
            "initialize" => self.handle_initialize().await,
            "extensions/list" => self.handle_extensions_list().await,
            "providers/list" => self.handle_providers_list().await,
            "speech/providers/list" => self.handle_speech_providers_list().await,
            "speech/synthesis/providers/list" => {
                self.handle_speech_synthesis_providers_list().await
            }
            "speech/synthesize" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_speech_synthesize(p).await
                })
                .await
            }
            "speech/transcribe" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_speech_transcribe(p).await
                })
                .await
            }
            "providers/configure" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_provider_configure(p).await
                })
                .await
            }
            "providers/clear" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_provider_clear(p).await
                })
                .await
            }
            "providers/select" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_provider_select(p).await
                })
                .await
            }
            "runners/list" => self.handle_runners_list().await,
            "runners/select" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_runners_select(p).await
                })
                .await
            }
            "runners/session" => self.handle_runners_session().await,
            "runners/snapshot" => self.handle_runners_snapshot().await,
            "runners/delete" => self.handle_runners_delete().await,
            "runners/ports" => self.handle_runners_ports().await,
            "settings/get" => self.handle_settings_get().await,
            "settings/set_web_search" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_web_search(p).await
                })
                .await
            }
            "settings/set_search_index" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_search_index(p).await
                })
                .await
            }
            "settings/set_shell" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_shell(p).await
                })
                .await
            }
            "search_index/status" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_search_index_status(p).await
                })
                .await
            }
            "search_index/warmup" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_search_index_warmup(p).await
                })
                .await
            }
            "search_index/rebuild" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_search_index_rebuild(p).await
                })
                .await
            }
            "search_index/clear" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_search_index_clear(p).await
                })
                .await
            }
            "index/status" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_code_index_status(p).await
                })
                .await
            }
            "index/rebuild" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_code_index_rebuild(p).await
                })
                .await
            }
            "index/search" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_code_index_search(p).await
                })
                .await
            }
            "index/readChunk" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_code_index_read_chunk(p).await
                })
                .await
            }
            "index/proofs/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_code_index_proofs_list(p).await
                })
                .await
            }
            "settings/set_default_mode" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_default_mode(p).await
                })
                .await
            }
            "settings/set_file_backed_dynamic_context" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_file_backed_dynamic_context(p)
                        .await
                })
                .await
            }
            "auth/codex/login" => self.handle_codex_auth_login().await,
            "auth/codex/status" => self.handle_codex_auth_status().await,
            "auth/codex/logout" => self.handle_codex_auth_logout().await,
            "auth/supergrok/login" => self.handle_supergrok_auth_login().await,
            "auth/supergrok/status" => self.handle_supergrok_auth_status().await,
            "auth/supergrok/logout" => self.handle_supergrok_auth_logout().await,
            "thread/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_thread_list(p).await },
                )
                .await
            }
            "thread/start" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_thread_start(p).await },
                )
                .await
            }
            "thread/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_thread_read(p).await },
                )
                .await
            }
            "thread/goal/get" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_goal_get(p).await
                })
                .await
            }
            "thread/goal/set" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_goal_set(p).await
                })
                .await
            }
            "thread/goal/clear" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_goal_clear(p).await
                })
                .await
            }
            "thread/archive" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_archive(p).await
                })
                .await
            }
            "roadmap/list" => self.handle_roadmap_list().await,
            "roadmap/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_roadmap_read(p).await },
                )
                .await
            }
            "roadmap/create" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_create(p).await
                })
                .await
            }
            "roadmap/patch" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_roadmap_patch(p).await },
                )
                .await
            }
            "roadmap/task/update" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_task_update(p).await
                })
                .await
            }
            "roadmap/validate" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_validate(p).await
                })
                .await
            }
            "roadmap/thread/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_thread_list(p).await
                })
                .await
            }
            "roadmap/thread/spawn" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_thread_spawn(p).await
                })
                .await
            }
            "roadmap/thread/attach" | "thread/attach" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_roadmap_thread_attach(p).await
                })
                .await
            }
            "thread/roadmap/open" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_roadmap_open(p).await
                })
                .await
            }
            "thread/state" => self.handle_thread_state().await,
            "thread/set_mode" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_set_mode(p).await
                })
                .await
            }
            "thread/exit_plan" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_exit_plan(p).await
                })
                .await
            }
            "thread/resolve_approval" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_resolve_approval(p).await
                })
                .await
            }
            "thread/resolve_user_input" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_resolve_user_input(p).await
                })
                .await
            }
            "turn/start" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_protocol_turn_start(p).await
                })
                .await
            }
            "turn/interrupt" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_protocol_turn_interrupt(p).await
                })
                .await
            }
            "turn/steer" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_protocol_turn_steer(p).await
                })
                .await
            }
            "team/start" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_team_start(p).await },
                )
                .await
            }
            "team/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_team_list(p).await },
                )
                .await
            }
            "team/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_team_read(p).await },
                )
                .await
            }
            "team/member/start" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_team_member_start(p).await
                })
                .await
            }
            "team/member/message" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_team_member_message(p).await
                })
                .await
            }
            "team/member/interrupt" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_team_member_interrupt(p).await
                })
                .await
            }
            "team/member/focus" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_team_member_focus(p).await
                })
                .await
            }
            "team/cleanup" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_team_cleanup(p).await },
                )
                .await
            }
            "team/pane/focus" | "team/pane/cleanup" => {
                Err(split_pane_unsupported_error(req.method.as_str()))
            }
            "model/list" => self.handle_model_list().await,
            "fs/readFile" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_fs_read_file(p).await },
                )
                .await
            }
            "fs/readDirectory" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_fs_read_directory(p).await
                })
                .await
            }
            "vcs/status" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_vcs_status(p).await },
                )
                .await
            }
            "vcs/changes/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_vcs_changes_list(p).await
                })
                .await
            }
            "vcs/changes/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_vcs_changes_read(p).await
                })
                .await
            }
            "vcs/select" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_vcs_select(p).await },
                )
                .await
            }
            "vcs/snapshot/create" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_vcs_snapshot_create(p).await
                })
                .await
            }
            "vcs/restore" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_vcs_restore(p).await },
                )
                .await
            }
            "vcs/lines/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_vcs_lines_list(p).await
                })
                .await
            }
            "vcs/lines/switch" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_vcs_lines_switch(p).await
                })
                .await
            }
            "vcs/sync" => {
                self.decode_and(req.params, |p| async move { self.handle_vcs_sync(p).await })
                    .await
            }
            "workspace/changes/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_changes_list(p).await
                })
                .await
            }
            "workspace/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_list(p).await
                })
                .await
            }
            "workspace/create" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_create(p).await
                })
                .await
            }
            "workspace/files/status" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_files_status(p).await
                })
                .await
            }
            "workspace/files/rebuild" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_files_rebuild(p).await
                })
                .await
            }
            "workspace/files/children" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_files_children(p).await
                })
                .await
            }
            "workspace/files/query" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_files_query(p).await
                })
                .await
            }
            "workspace/files/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_files_read(p).await
                })
                .await
            }
            "workspace/update" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_update(p).await
                })
                .await
            }
            "workspace/forget" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workspace_forget(p).await
                })
                .await
            }
            "command/exec" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_command_exec(p).await },
                )
                .await
            }
            "tools/list" => self.handle_tools_list().await,
            "discovery/groups" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_groups(p).await
                })
                .await
            }
            "discovery/search" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_search(p).await
                })
                .await
            }
            "discovery/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_read(p).await
                })
                .await
            }
            "discovery/refresh" => self.handle_discovery_refresh().await,
            "discovery/promote" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_promote(p).await
                })
                .await
            }
            "discovery/promoted/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_promoted_list(p).await
                })
                .await
            }
            "discovery/promoted/clear" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_discovery_promoted_clear(p).await
                })
                .await
            }
            "retrieval/recommendations" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_retrieval_recommendations(p).await
                })
                .await
            }
            "retrieval/metrics" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_retrieval_metrics(p).await
                })
                .await
            }
            "retrieval/promoted" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_retrieval_promoted(p).await
                })
                .await
            }
            "tasks/submit" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_tasks_submit(p).await },
                )
                .await
            }
            "tasks/list" => self.handle_tasks_list().await,
            "tasks/get" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_tasks_get(p).await },
                )
                .await
            }
            "tasks/cancel" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_tasks_cancel(p).await },
                )
                .await
            }
            "tasks/subscribe" => self.handle_tasks_subscribe().await,
            "webwright/prepare" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_prepare(p).await
                })
                .await
            }
            "webwright/artifacts" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_artifacts(p).await
                })
                .await
            }
            "webwright/latestRun" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_latest_run(p).await
                })
                .await
            }
            "webwright/report" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_report(p).await
                })
                .await
            }
            "webwright/verify" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_verify(p).await
                })
                .await
            }
            "webwright/visualJudge" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_visual_judge(p).await
                })
                .await
            }
            "webwright/export" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_export(p).await
                })
                .await
            }
            "webwright/setup" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_setup(p).await
                })
                .await
            }
            "webwright/submit" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_submit(p).await
                })
                .await
            }
            "webwright/rerun" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_webwright_rerun(p).await
                })
                .await
            }
            "processes/list" => {
                match req
                    .params
                    .map(serde_json::from_value::<ProcessesListParams>)
                    .transpose()
                    .map_err(invalid_params)
                {
                    Ok(params) => self.handle_processes_list(params.unwrap_or_default()).await,
                    Err(err) => Err(err),
                }
            }
            "processes/get" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_processes_get(p).await },
                )
                .await
            }
            "processes/stop" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_processes_stop(p).await
                })
                .await
            }
            "processes/stopAll" => {
                match req
                    .params
                    .map(serde_json::from_value::<ProcessesStopAllParams>)
                    .transpose()
                    .map_err(invalid_params)
                {
                    Ok(params) => {
                        self.handle_processes_stop_all(params.unwrap_or_default())
                            .await
                    }
                    Err(err) => Err(err),
                }
            }
            "processes/subscribe" => self.handle_processes_subscribe().await,
            "automations/list" => {
                match req
                    .params
                    .map(serde_json::from_value::<AutomationsListParams>)
                    .transpose()
                    .map_err(invalid_params)
                {
                    Ok(params) => {
                        self.handle_automations_list(params.unwrap_or_default())
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            "automations/create" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_create(p).await
                })
                .await
            }
            "automations/update" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_update(p).await
                })
                .await
            }
            "automations/delete" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_delete(p).await
                })
                .await
            }
            "automations/runNow" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_run_now(p).await
                })
                .await
            }
            "automations/runs" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_runs(p).await
                })
                .await
            }
            "automations/cancelRun" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_automations_cancel_run(p).await
                })
                .await
            }
            "automations/status" => self.handle_automations_status().await,
            "commands/list" => self.handle_commands_list().await,
            "skills/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_skills_list(p).await },
                )
                .await
            }
            "skills/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_skills_read(p).await },
                )
                .await
            }
            "skills/setEnabled" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_skills_set_enabled(p).await
                })
                .await
            }
            "skills/setExposure" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_skills_set_exposure(p).await
                })
                .await
            }
            "eval/reports/list" => {
                self.decode_and(req.params, |p| async move {
                    crate::evals::handle_eval_reports_list(&self.runtime.workspace(), p)
                })
                .await
            }
            "eval/report/read" => {
                self.decode_and(req.params, |p| async move {
                    crate::evals::handle_eval_report_read(&self.runtime.workspace(), p)
                })
                .await
            }
            "commands/expand" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_commands_expand(p).await
                })
                .await
            }
            "commands/run" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_commands_run(p).await },
                )
                .await
            }
            "tools/call" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_tool_call(p).await },
                )
                .await
            }
            "agents/list" => self.handle_agents_list().await,
            "turn/subagentTraces/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_subagent_traces_list(p).await
                })
                .await
            }
            "turn/subagentTrace/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_subagent_trace_read(p).await
                })
                .await
            }
            "plan/review/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plan_review_read(p).await
                })
                .await
            }
            "plan/review/comment" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plan_review_comment(p).await
                })
                .await
            }
            "plan/review/rewrite" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plan_review_rewrite(p).await
                })
                .await
            }
            "plan/review/approve" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plan_review_approve(p).await
                })
                .await
            }
            "plan/review/reject" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plan_review_reject(p).await
                })
                .await
            }
            "hunk/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_hunk_list(p).await },
                )
                .await
            }
            "hunk/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_hunk_read(p).await },
                )
                .await
            }
            "hunk/rollback" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_hunk_rollback(p).await },
                )
                .await
            }
            "workflow/scan" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_workflow_scan(p).await },
                )
                .await
            }
            "workflow/preview" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflow_preview(p).await
                })
                .await
            }
            "workflow/enable" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflow_enable(p).await
                })
                .await
            }
            "workflow/ignore" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflow_ignore(p).await
                })
                .await
            }
            "workflow/refresh" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflow_refresh(p).await
                })
                .await
            }
            "workflow/remove" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflow_remove(p).await
                })
                .await
            }
            "workflows/plan" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_plan(p).await
                })
                .await
            }
            "workflows/approve" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_approve(p).await
                })
                .await
            }
            "workflows/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_list(p).await
                })
                .await
            }
            "workflows/get" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_workflows_get(p).await },
                )
                .await
            }
            "workflows/pause" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_pause(p).await
                })
                .await
            }
            "workflows/resume" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_resume(p).await
                })
                .await
            }
            "workflows/stop" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_stop(p).await
                })
                .await
            }
            "workflows/restartAgent" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_restart_agent(p).await
                })
                .await
            }
            "workflows/save" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_save(p).await
                })
                .await
            }
            "workflows/scripts/list" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_scripts_list(p).await
                })
                .await
            }
            "workflows/scripts/read" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_scripts_read(p).await
                })
                .await
            }
            "workflows/scripts/delete" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_workflows_scripts_delete(p).await
                })
                .await
            }
            "marketplaces/list" => self.handle_marketplaces_list().await,
            "marketplaces/install_default" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplaces_install_default(p).await
                })
                .await
            }
            "marketplaces/add" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplaces_add(p).await
                })
                .await
            }
            "marketplaces/remove" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplaces_remove(p).await
                })
                .await
            }
            "marketplaces/refresh" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplaces_refresh(p).await
                })
                .await
            }
            "marketplaces/search" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplaces_search(p).await
                })
                .await
            }
            "marketplaces/plugin" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_marketplace_plugin(p).await
                })
                .await
            }
            "plugins/preview_install" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plugins_preview_install(p).await
                })
                .await
            }
            "plugins/install" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plugins_install(p).await
                })
                .await
            }
            "plugins/install_all_variants" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plugins_install_all_variants(p).await
                })
                .await
            }
            "plugins/list_installed" => self.handle_plugins_list_installed().await,
            "plugins/disable" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plugins_disable(p).await
                })
                .await
            }
            "plugins/uninstall" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_plugins_uninstall(p).await
                })
                .await
            }
            "media/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_media_list(p).await },
                )
                .await
            }
            "media/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_media_read(p).await },
                )
                .await
            }
            "media/thumbnail" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_media_thumbnail(p).await
                })
                .await
            }
            "media/delete" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_media_delete(p).await },
                )
                .await
            }
            "media/attachToTurn" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_media_attach_to_turn(p).await
                })
                .await
            }
            "artifact/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_artifact_list(p).await },
                )
                .await
            }
            "artifact/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_artifact_read(p).await },
                )
                .await
            }
            "artifact/grep" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_artifact_grep(p).await },
                )
                .await
            }
            "artifact/tail" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_artifact_tail(p).await },
                )
                .await
            }
            "artifact/delete" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_artifact_delete(p).await
                })
                .await
            }
            "memory/list" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_list(p).await },
                )
                .await
            }
            "memory/read" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_read(p).await },
                )
                .await
            }
            "memory/save" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_save(p).await },
                )
                .await
            }
            "memory/update" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_update(p).await },
                )
                .await
            }
            "memory/delete" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_delete(p).await },
                )
                .await
            }
            "memory/query" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_memory_query(p).await },
                )
                .await
            }
            "memory/provider/list" => self.handle_memory_provider_list().await,
            "memory/provider/set" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_memory_provider_set(p).await
                })
                .await
            }
            "memory/recall/preview" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_memory_recall_preview(p).await
                })
                .await
            }
            _ => Err(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            }),
        };

        match result {
            Ok(val) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(val),
                error: None,
            },
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: None,
                error: Some(err),
            },
        }
    }

    async fn decode_and<T, F, Fut>(
        &self,
        params: Option<serde_json::Value>,
        f: F,
    ) -> Result<serde_json::Value, JsonRpcError>
    where
        T: serde::de::DeserializeOwned,
        F: FnOnce(T) -> Fut,
        Fut: std::future::Future<Output = Result<serde_json::Value, JsonRpcError>>,
    {
        let Some(params) = params else {
            return Err(JsonRpcError {
                code: -32602,
                message: "Missing params".to_string(),
                data: None,
            });
        };
        let params = serde_json::from_value::<T>(params).map_err(invalid_params)?;
        f(params).await
    }

    async fn handle_initialize(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(InitializeResult {
            provider: cfg.default_provider,
            model: cfg.default_model,
            cwd: std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string()),
        })
        .unwrap())
    }

    async fn handle_runners_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let providers = self
            .runtime
            .registry()
            .remote_runner_providers
            .iter()
            .map(|provider| RunnerProviderDescriptor {
                provider_id: provider.id(),
                capabilities: provider.capabilities(),
            })
            .collect::<Vec<_>>();
        Ok(serde_json::to_value(RunnersListResult {
            active: runner_status(cfg.remote_runner_destination.as_ref(), None),
            providers,
        })
        .unwrap())
    }

    async fn handle_runners_select(
        &self,
        params: RunnersSelectParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let provider_id = params
            .provider_id
            .unwrap_or_else(|| params.destination_id.clone());
        let Some(provider) = self
            .runtime
            .registry()
            .remote_runner_providers
            .iter()
            .find(|provider| provider.id() == provider_id)
        else {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("unknown runner provider {provider_id:?}"),
                data: None,
            });
        };
        let destination = roder_api::remote_runner::RunnerDestination {
            id: params.destination_id,
            provider_id,
            config: params.config,
            default_manifest: params.manifest,
        };
        provider
            .validate_destination(&destination)
            .await
            .map_err(invalid_params_error)?;
        self.runtime
            .set_remote_runner_destination(Some(destination.clone()))
            .await;
        Ok(serde_json::to_value(RunnersSelectResult {
            active: runner_status(Some(&destination), None),
        })
        .unwrap())
    }

    async fn handle_runners_session(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(RunnersSessionResult {
            active: runner_status(cfg.remote_runner_destination.as_ref(), None),
        })
        .unwrap())
    }

    async fn handle_runners_snapshot(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(RunnersSnapshotResult { snapshot: None }).unwrap())
    }

    async fn handle_runners_delete(&self) -> Result<serde_json::Value, JsonRpcError> {
        self.runtime.set_remote_runner_destination(None).await;
        Ok(serde_json::to_value(RunnersDeleteResult { deleted: true }).unwrap())
    }

    async fn handle_runners_ports(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(RunnersPortsResult { ports: Vec::new() }).unwrap())
    }

    async fn handle_extensions_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ExtensionsListResult {
            extensions: self.runtime.registry().manifests.clone(),
            capability_statuses: self.runtime.registry().capability_statuses.clone(),
        })
        .unwrap())
    }

    async fn handle_providers_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let mut providers = Vec::new();
        for engine in &self.runtime.registry().inference_engines {
            let id = engine.id();
            let metadata = engine.metadata();
            let (authenticated, auth_detail) = provider_auth_status(&id, &metadata).await;
            let models = engine
                .list_models(InferenceProviderContext { provider_id: &id })
                .await
                .unwrap_or_default();
            providers.push(ProviderDescriptor {
                id,
                name: metadata.name,
                description: metadata.description,
                auth_type: metadata.auth_type,
                auth_label: metadata.auth_label,
                authenticated,
                auth_detail,
                recommended: metadata.recommended,
                sort_order: metadata.sort_order,
                capabilities: engine.capabilities(),
                models,
            });
        }
        providers.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(serde_json::to_value(ProvidersListResult {
            active_provider: cfg.default_provider,
            active_model: cfg.default_model,
            active_reasoning: self.runtime.effective_reasoning().await,
            providers,
        })
        .unwrap())
    }

    async fn handle_model_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let mut models = Vec::new();
        for engine in &self.runtime.registry().inference_engines {
            let provider_id = engine.id();
            let provider_models = engine
                .list_models(InferenceProviderContext {
                    provider_id: &provider_id,
                })
                .await
                .unwrap_or_default();
            for model in provider_models {
                let model_id = model.id;
                models.push(Model {
                    is_default: provider_id == cfg.default_provider
                        && model_id == cfg.default_model,
                    id: model_id,
                    name: model.name,
                    model_provider: provider_id.clone(),
                    default_reasoning_effort: model.default_reasoning,
                    reasoning_efforts: model
                        .supported_reasoning
                        .into_iter()
                        .map(|effort| effort.effort)
                        .collect(),
                });
            }
        }
        Ok(serde_json::to_value(ModelListResult { models }).unwrap())
    }

    async fn handle_provider_select(
        &self,
        params: ProviderSelectParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let thread_id = params.thread_id.clone();
        let previous_model = if let Some(thread_id) = thread_id.as_deref() {
            self.protocol_thread_model(thread_id).await
        } else {
            None
        };
        let cfg = if thread_id.is_some() {
            self.runtime
                .preview_provider_selection(params.provider, params.model, params.reasoning)
                .await
                .map_err(internal_error)?
        } else {
            self.runtime
                .select_provider(params.provider, params.model, params.reasoning)
                .await
                .map_err(internal_error)?
        };
        let model_profile = active_model_profile_label(&cfg);
        let model_switch_summary = previous_model
            .filter(|selection| {
                selection.provider != cfg.default_provider || selection.model != cfg.default_model
            })
            .map(|selection| {
                format!(
                    "Model switch summary: previous profile {}/{}. Current profile {}/{}.",
                    selection.provider, selection.model, cfg.default_provider, model_profile
                )
            });
        let reasoning = Runtime::effective_reasoning_for_config(&cfg);
        if let Some(thread_id) = thread_id.as_ref() {
            self.protocol_thread_models.write().await.insert(
                thread_id.clone(),
                ProtocolThreadModelSelection {
                    provider: cfg.default_provider.clone(),
                    model: cfg.default_model.clone(),
                    reasoning: Some(reasoning.clone()),
                },
            );
            if let Some(thread) = self.protocol_threads.write().await.get_mut(thread_id) {
                thread.model_provider = cfg.default_provider.clone();
                thread.model = cfg.default_model.clone();
                thread.updated_at = OffsetDateTime::now_utc().unix_timestamp();
            }
        }
        if thread_id.is_none() && self.persist_user_config {
            roder_config::save_default_provider_model_reasoning(
                &cfg.default_provider,
                &cfg.default_model,
                cfg.reasoning.as_deref(),
            )
            .map_err(internal_error)?;
        }
        Ok(serde_json::to_value(ProviderSelectResult {
            provider: cfg.default_provider,
            model: cfg.default_model,
            reasoning,
            model_profile: Some(model_profile),
            model_switch_summary,
        })
        .unwrap())
    }

    async fn handle_provider_configure(
        &self,
        params: ProviderConfigureParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let provider = roder_api::catalog::normalize_provider_id(params.provider.trim());
        let api_key = params.api_key.trim();
        if provider.is_empty() {
            return Err(invalid_params("provider is required"));
        }
        if api_key.is_empty() {
            return Err(invalid_params("api_key is required"));
        }
        if !self
            .runtime
            .registry()
            .inference_engines
            .iter()
            .any(|engine| engine.id() == provider)
        {
            return Err(invalid_params(format!("unknown provider {provider:?}")));
        }
        if !self.persist_user_config {
            return Err(internal_error(
                "provider API key persistence is disabled for this app-server",
            ));
        }
        roder_config::save_provider_api_key(&provider, api_key).map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderConfigureResult {
            provider,
            authenticated: true,
        })
        .unwrap())
    }

    async fn handle_provider_clear(
        &self,
        params: ProviderClearParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let provider = roder_api::catalog::normalize_provider_id(params.provider.trim());
        if provider.is_empty() {
            return Err(invalid_params("provider is required"));
        }
        if !self.persist_user_config {
            return Err(internal_error(
                "provider API key persistence is disabled for this app-server",
            ));
        }
        roder_config::delete_provider_api_key(&provider).map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderClearResult { provider }).unwrap())
    }

    async fn handle_settings_get(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(SettingsGetResult {
            web_search: WebSearchSettings {
                mode: cfg.hosted_web_search.mode,
            },
            search_index: SearchIndexSettings {
                enabled: roder_search::search_index_enabled(),
            },
            shell: shell_settings(&cfg.command_shell),
            default_provider: cfg.default_provider.clone(),
            default_model: cfg.default_model.clone(),
            default_reasoning: Runtime::effective_reasoning_for_config(&cfg),
            default_mode: cfg.policy_mode,
            file_backed_dynamic_context: cfg.file_backed_dynamic_context,
        })
        .unwrap())
    }

    async fn handle_settings_set_web_search(
        &self,
        params: SettingsSetWebSearchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .set_hosted_web_search(params.mode)
            .await
            .map_err(internal_error)?;
        if self.persist_user_config {
            roder_config::save_web_search_mode(web_search_mode_config_value(
                cfg.hosted_web_search.mode,
            ))
            .map_err(internal_error)?;
        }
        Ok(serde_json::to_value(SettingsSetWebSearchResult {
            web_search: WebSearchSettings {
                mode: cfg.hosted_web_search.mode,
            },
        })
        .unwrap())
    }

    async fn handle_settings_set_search_index(
        &self,
        params: SettingsSetSearchIndexParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        roder_search::set_search_index_enabled(params.enabled);
        if self.persist_user_config {
            roder_config::save_search_index_enabled(params.enabled).map_err(internal_error)?;
        }
        let status = self.search_index_status(None);
        self.publish_search_index_status(status);
        Ok(serde_json::to_value(SettingsSetSearchIndexResult {
            search_index: SearchIndexSettings {
                enabled: params.enabled,
            },
        })
        .unwrap())
    }

    async fn handle_settings_set_shell(
        &self,
        params: SettingsSetShellParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let shell = roder_api::command_shell::normalize_command_shell(&params.shell)
            .ok_or_else(|| invalid_params("shell is required"))?;
        let cfg = self.runtime.set_command_shell(shell).await;
        if self.persist_user_config {
            roder_config::save_tools_shell(&cfg.command_shell).map_err(internal_error)?;
        }
        Ok(serde_json::to_value(SettingsSetShellResult {
            shell: shell_settings(&cfg.command_shell),
        })
        .unwrap())
    }

    async fn handle_settings_set_default_mode(
        &self,
        params: SettingsSetDefaultModeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .set_policy_mode(params.mode, Some("settings default mode".to_string()))
            .await
            .map_err(internal_error)?;
        if self.persist_user_config {
            roder_config::save_default_policy_mode(policy_mode_config_value(cfg.policy_mode))
                .map_err(internal_error)?;
        }
        Ok(serde_json::to_value(SettingsSetDefaultModeResult {
            default_mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn handle_settings_set_file_backed_dynamic_context(
        &self,
        params: SettingsSetFileBackedDynamicContextParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .set_file_backed_dynamic_context(params.enabled)
            .await;
        if self.persist_user_config {
            roder_config::save_file_backed_dynamic_context(cfg.file_backed_dynamic_context)
                .map_err(internal_error)?;
        }
        Ok(
            serde_json::to_value(SettingsSetFileBackedDynamicContextResult {
                enabled: cfg.file_backed_dynamic_context,
            })
            .unwrap(),
        )
    }

    async fn handle_codex_auth_login(&self) -> Result<serde_json::Value, JsonRpcError> {
        if !self.persist_user_config {
            return Err(internal_error(
                "codex auth persistence is disabled for this app-server",
            ));
        }
        let tokens = roder_codex_auth::login().await.map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: true,
            account_id: non_empty(tokens.account_id),
        })
        .unwrap())
    }

    async fn handle_codex_auth_status(&self) -> Result<serde_json::Value, JsonRpcError> {
        let signed_in = roder_codex_auth::status().await.map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: signed_in.is_some(),
            account_id: signed_in.and_then(|tokens| non_empty(tokens.account_id)),
        })
        .unwrap())
    }

    async fn handle_codex_auth_logout(&self) -> Result<serde_json::Value, JsonRpcError> {
        if !self.persist_user_config {
            return Err(internal_error(
                "codex auth persistence is disabled for this app-server",
            ));
        }
        roder_codex_auth::logout().map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: false,
            account_id: None,
        })
        .unwrap())
    }

    async fn handle_supergrok_auth_login(&self) -> Result<serde_json::Value, JsonRpcError> {
        if !self.persist_user_config {
            return Err(internal_error(
                "supergrok auth persistence is disabled for this app-server",
            ));
        }
        let tokens = roder_supergrok_auth::login()
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: true,
            account_id: non_empty(tokens.email),
        })
        .unwrap())
    }

    async fn handle_supergrok_auth_status(&self) -> Result<serde_json::Value, JsonRpcError> {
        let signed_in = roder_supergrok_auth::status()
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: signed_in.is_some(),
            account_id: signed_in.and_then(|tokens| non_empty(tokens.email)),
        })
        .unwrap())
    }

    async fn handle_supergrok_auth_logout(&self) -> Result<serde_json::Value, JsonRpcError> {
        if !self.persist_user_config {
            return Err(internal_error(
                "supergrok auth persistence is disabled for this app-server",
            ));
        }
        roder_supergrok_auth::logout().map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: false,
            account_id: None,
        })
        .unwrap())
    }

    async fn live_thread_status(&self, thread_id: &str) -> ThreadStatus {
        let thread_id = thread_id.to_string();
        let activity = self.runtime.thread_activity(&thread_id).await;
        thread_status_for_activity(activity.active_turn_id, activity.active_flags)
    }

    async fn protocol_thread_from_metadata_with_live_status(
        &self,
        metadata: roder_api::thread::ThreadMetadata,
        turns: Option<Vec<Turn>>,
    ) -> Thread {
        let status = self.live_thread_status(&metadata.thread_id).await;
        protocol_thread_from_metadata(metadata, turns, status)
    }

    async fn thread_with_live_status(&self, mut thread: Thread) -> Thread {
        thread.status = self.live_thread_status(&thread.id).await;
        thread
    }

    async fn handle_thread_list(
        &self,
        params: ThreadListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut thread_metadata = self.runtime.list_threads().await.map_err(internal_error)?;
        thread_metadata.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        if let Some(limit) = params.limit {
            thread_metadata.truncate(limit);
        }
        let mut threads = Vec::new();
        for metadata in thread_metadata {
            threads.push(
                self.protocol_thread_from_metadata_with_live_status(metadata, None)
                    .await,
            );
        }
        let protocol_threads = self
            .protocol_threads
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for thread in protocol_threads {
            if !threads.iter().any(|candidate| candidate.id == thread.id) {
                threads.push(self.thread_with_live_status(thread).await);
            }
        }
        Ok(serde_json::to_value(ThreadListResult {
            data: threads,
            next_cursor: None,
            backwards_cursor: None,
        })
        .unwrap())
    }

    async fn handle_thread_start(
        &self,
        params: ThreadStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let model = params
            .model
            .clone()
            .unwrap_or_else(|| cfg.default_model.clone());
        let model_provider = params
            .model_provider
            .clone()
            .unwrap_or_else(|| cfg.default_provider.clone());
        let resolved = self
            .workspaces
            .resolve_root(
                cfg.workspace.clone(),
                &params.workspace_id,
                params.root_id.as_deref(),
            )
            .await?;
        let cwd = crate::workspaces::validate_cwd(&resolved.root, params.cwd.clone())?;
        let metadata = self
            .runtime
            .create_thread_with(roder_core::CreateThreadRequest {
                title: None,
                workspace: cwd.clone(),
                workspace_id: Some(resolved.workspace.id.clone()),
                root_id: Some(resolved.root.id.clone()),
                provider: params.model_provider.clone(),
                model: params.model.clone(),
            })
            .await
            .map_err(internal_error)?;
        let reasoning = params
            .reasoning
            .clone()
            .unwrap_or_else(|| Runtime::effective_reasoning_for_config(&cfg));
        let thread = protocol_thread_from_metadata(metadata, None, idle_thread_status());
        self.protocol_threads
            .write()
            .await
            .insert(thread.id.clone(), thread.clone());
        self.protocol_thread_models.write().await.insert(
            thread.id.clone(),
            ProtocolThreadModelSelection {
                provider: model_provider.clone(),
                model: model.clone(),
                reasoning: Some(reasoning.clone()),
            },
        );
        let _ = self
            .protocol_notifications
            .send(notifications::thread_started_notification(thread.clone()));
        Ok(serde_json::to_value(ThreadStartResult {
            thread,
            model,
            model_provider,
            reasoning,
            cwd,
            workspace_id: resolved.workspace.id,
            root_id: resolved.root.id,
        })
        .unwrap())
    }

    async fn handle_thread_read(
        &self,
        params: ThreadReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let thread = snapshot.and_then(|snapshot| {
            let turns = params
                .include_turns
                .then(|| protocol_turns_from_snapshot(&snapshot));
            snapshot.metadata.map(|metadata| (metadata, turns))
        });
        let thread = match thread {
            Some((metadata, turns)) => Some(
                self.protocol_thread_from_metadata_with_live_status(metadata, turns)
                    .await,
            ),
            None => None,
        };
        let thread = if thread.is_some() {
            thread
        } else {
            let metadata = self
                .runtime
                .list_threads()
                .await
                .map_err(internal_error)?
                .into_iter()
                .find(|metadata| metadata.thread_id == params.thread_id);
            match metadata {
                Some(metadata) => Some(
                    self.protocol_thread_from_metadata_with_live_status(
                        metadata,
                        params.include_turns.then(Vec::new),
                    )
                    .await,
                ),
                None => None,
            }
        };
        let thread = if thread.is_some() {
            thread
        } else {
            let thread = self
                .protocol_threads
                .read()
                .await
                .get(params.thread_id.as_str())
                .cloned();
            match thread {
                Some(mut thread) => {
                    if params.include_turns && thread.turns.is_none() {
                        thread.turns = Some(Vec::new());
                    }
                    Some(self.thread_with_live_status(thread).await)
                }
                None => None,
            }
        };
        Ok(serde_json::to_value(ThreadReadResult { thread }).unwrap())
    }

    async fn handle_thread_archive(
        &self,
        params: ThreadArchiveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let archived = self
            .runtime
            .archive_thread(&params.thread_id)
            .await
            .map_err(internal_error)?;
        self.protocol_threads
            .write()
            .await
            .remove(&params.thread_id);
        self.protocol_thread_models
            .write()
            .await
            .remove(&params.thread_id);
        Ok(serde_json::to_value(ThreadArchiveResult {
            thread_id: params.thread_id,
            archived,
        })
        .unwrap())
    }

    async fn handle_roadmap_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let documents = self.runtime.list_roadmaps().await.map_err(internal_error)?;
        Ok(serde_json::json!({ "documents": documents }))
    }

    async fn handle_roadmap_read(
        &self,
        params: RoadmapPathParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let path = self.roadmap_path(&params.path)?;
        let content = std::fs::read_to_string(&path).map_err(internal_error)?;
        let document = parse_document(&path, &content);
        Ok(serde_json::json!({ "document": document }))
    }

    async fn handle_roadmap_create(
        &self,
        params: RoadmapCreateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.roadmap_workspace()?;
        let roadmap_dir = workspace.join("roadmap");
        std::fs::create_dir_all(&roadmap_dir).map_err(internal_error)?;
        let slug = sanitize_roadmap_slug(&params.slug)?;
        let phase = next_roadmap_phase(&roadmap_dir).map_err(internal_error)?;
        let path = roadmap_dir.join(format!("{phase:02}-{slug}.md"));
        if path.exists() {
            return Err(invalid_params(format!(
                "roadmap already exists: {}",
                path.display()
            )));
        }
        let goal = params
            .goal
            .as_deref()
            .unwrap_or("Describe the intended outcome.");
        std::fs::write(&path, roadmap_template(&params.title, goal, phase, &slug))
            .map_err(internal_error)?;
        let content = std::fs::read_to_string(&path).map_err(internal_error)?;
        let document = parse_document(&path, &content);
        Ok(serde_json::json!({ "document": document, "path": rel(&workspace, &path) }))
    }

    async fn handle_roadmap_patch(
        &self,
        params: RoadmapPatchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let path = self.roadmap_path(&params.path)?;
        let content = std::fs::read_to_string(&path).map_err(internal_error)?;
        if !content.contains(&params.old_text) {
            return Err(not_found("roadmap patch oldText not found"));
        }
        let updated = content.replacen(&params.old_text, &params.new_text, 1);
        std::fs::write(&path, updated).map_err(internal_error)?;
        Ok(serde_json::json!({ "path": self.roadmap_rel(&path)?, "changed": true }))
    }

    async fn handle_roadmap_task_update(
        &self,
        params: RoadmapTaskUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let evidence = params.evidence.unwrap_or_default();
        self.runtime
            .set_roadmap_task(&params.path, &params.task_id, params.checked, &evidence)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({
            "path": params.path,
            "taskId": params.task_id,
            "checked": params.checked
        }))
    }

    async fn handle_roadmap_validate(
        &self,
        params: RoadmapValidateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if let Some(path) = params.path {
            let validation = self
                .runtime
                .validate_roadmap(&path)
                .await
                .map_err(internal_error)?;
            return Ok(serde_json::json!({ "results": [validation] }));
        }
        let workspace = self.roadmap_workspace()?;
        let mut results = Vec::new();
        for document in
            list_documents(&workspace, ListOptions::default()).map_err(internal_error)?
        {
            let content = std::fs::read_to_string(&document.path).map_err(internal_error)?;
            let document = parse_document(&document.path, &content);
            results.push(validate_document(&document));
        }
        Ok(serde_json::json!({ "results": results }))
    }

    async fn handle_roadmap_thread_list(
        &self,
        params: RoadmapPathParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let threads = self
            .runtime
            .list_roadmap_threads(&params.path)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({ "threads": threads }))
    }

    async fn handle_roadmap_thread_spawn(
        &self,
        params: RoadmapThreadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let task_id = params
            .task_id
            .ok_or_else(|| invalid_params("roadmap/thread/spawn requires taskId"))?;
        let cfg = self.runtime.status().await;
        let metadata = self
            .runtime
            .create_thread_with(CreateThreadRequest {
                title: Some(format!("Roadmap worker: {task_id}")),
                workspace: self.roadmap_workspace()?.display().to_string(),
                workspace_id: None,
                root_id: None,
                provider: Some(cfg.default_provider.clone()),
                model: Some(cfg.default_model.clone()),
            })
            .await
            .map_err(internal_error)?;
        let protocol_thread = protocol_thread_from_metadata(metadata, None, idle_thread_status());
        self.protocol_threads
            .write()
            .await
            .insert(protocol_thread.id.clone(), protocol_thread.clone());
        let reasoning = Runtime::effective_reasoning_for_config(&cfg);
        self.protocol_thread_models.write().await.insert(
            protocol_thread.id.clone(),
            ProtocolThreadModelSelection {
                provider: cfg.default_provider,
                model: cfg.default_model,
                reasoning: Some(reasoning),
            },
        );
        let _ = self
            .protocol_notifications
            .send(notifications::thread_started_notification(
                protocol_thread.clone(),
            ));
        let thread = self
            .runtime
            .attach_roadmap_thread(
                &params.path,
                &task_id,
                &protocol_thread.id,
                params
                    .title
                    .or_else(|| Some(format!("Roadmap worker: {task_id}"))),
            )
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({ "thread": thread }))
    }

    async fn handle_roadmap_thread_attach(
        &self,
        params: RoadmapThreadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let task_id = params
            .task_id
            .ok_or_else(|| invalid_params("roadmap thread attach requires taskId"))?;
        let thread_id = params
            .thread_id
            .ok_or_else(|| invalid_params("roadmap thread attach requires threadId"))?;
        let thread = self
            .runtime
            .attach_roadmap_thread(&params.path, &task_id, &thread_id, params.title)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({ "thread": thread }))
    }

    async fn handle_thread_roadmap_open(
        &self,
        params: RoadmapPathParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let document = self
            .runtime
            .open_roadmap(&params.path)
            .await
            .map_err(internal_error)?;
        self.runtime
            .enter_roadmap_mode(&params.path)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({ "document": document }))
    }

    fn roadmap_workspace(&self) -> Result<std::path::PathBuf, JsonRpcError> {
        Ok(self.runtime.workspace())
    }

    fn roadmap_path(&self, path: &str) -> Result<std::path::PathBuf, JsonRpcError> {
        let workspace = self.roadmap_workspace()?;
        resolve_roadmap_path(&workspace, path).map_err(invalid_params)
    }

    fn roadmap_rel(&self, path: &std::path::Path) -> Result<String, JsonRpcError> {
        Ok(rel(&self.roadmap_workspace()?, path))
    }

    async fn handle_thread_state(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let pending =
            self.runtime
                .pending_plan_exit()
                .await
                .map(|pending| PendingPlanExitDescriptor {
                    thread_id: pending.thread_id,
                    turn_id: pending.turn_id,
                    request_id: pending.request_id,
                    target_mode: pending.target_mode,
                    plan_summary: pending.plan_summary,
                    requested_at: pending.requested_at,
                    expires_at: pending.expires_at,
                });
        Ok(serde_json::to_value(ThreadStateResult {
            mode: cfg.policy_mode,
            pending_plan_exit: pending,
        })
        .unwrap())
    }

    async fn handle_thread_set_mode(
        &self,
        params: ThreadSetModeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .set_policy_mode(params.mode, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadSetModeResult {
            mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn handle_thread_exit_plan(
        &self,
        params: ThreadExitPlanParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_pending_plan_exit(&params.request_id, params.approved)
            .await
            .map_err(internal_error)?
            .is_some();
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(ThreadExitPlanResult {
            resolved,
            mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn handle_thread_resolve_approval(
        &self,
        params: ThreadResolveApprovalParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_tool_approval(&params.approval_id, params.approved)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadResolveApprovalResult { resolved }).unwrap())
    }

    async fn handle_thread_resolve_user_input(
        &self,
        params: ThreadResolveUserInputParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_user_input(&params.request_id, params.answers)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadResolveUserInputResult { resolved }).unwrap())
    }

    async fn handle_protocol_turn_start(
        &self,
        params: TurnStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let message = protocol_turn_message(&params.input, params.prompt);
        let images = protocol_turn_images(&params.input);
        if let Some(turn_id) = self.runtime.active_turn_for_thread(&params.thread_id).await {
            self.runtime
                .steer_turn(params.thread_id.clone(), turn_id.clone(), message, images)
                .await
                .map_err(internal_error)?;
            return Ok(serde_json::to_value(TurnStartResult { turn_id }).unwrap());
        }

        let (thread_provider, thread_model, thread_reasoning) = self
            .protocol_thread_model(&params.thread_id)
            .await
            .map(|selection| {
                (
                    Some(selection.provider),
                    Some(selection.model),
                    selection.reasoning,
                )
            })
            .unwrap_or((None, None, None));
        let provider_override = params.model_provider.or(thread_provider);
        let model_override = params.model.or(thread_model);
        let reasoning_override = params.reasoning.or(thread_reasoning);
        let protocol_thread_workspace = self
            .protocol_threads
            .read()
            .await
            .get(&params.thread_id)
            .map(|thread| thread.cwd.clone());
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let workspace = if let Some(metadata) = snapshot.and_then(|snapshot| snapshot.metadata) {
            metadata.workspace
        } else if let Some(workspace) = protocol_thread_workspace {
            workspace
        } else {
            return Err(not_found(format!("thread not found: {}", params.thread_id)));
        };
        if let Some(policy_mode) = params.policy_mode {
            self.runtime
                .set_policy_mode(
                    policy_mode,
                    Some("turn/start selected policy mode".to_string()),
                )
                .await
                .map_err(internal_error)?;
        }
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id.clone(),
                message,
                images,
                provider_override,
                model_override,
                reasoning_override,
                workspace,
                instructions: default_instructions(),
                task_ledger_required: params.task_ledger_required,
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TurnStartResult { turn_id }).unwrap())
    }

    async fn handle_protocol_turn_interrupt(
        &self,
        params: TurnInterruptParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = if let Some(turn_id) = params.turn_id.clone() {
            turn_id
        } else {
            self.runtime
                .active_turn_for_thread(&params.thread_id)
                .await
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: format!("no active turn for thread {:?}", params.thread_id),
                    data: None,
                })?
        };
        self.runtime
            .interrupt_turn(params.thread_id.clone(), turn_id.clone())
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TurnInterruptResult {
            turn_id: Some(turn_id),
        })
        .unwrap())
    }

    async fn handle_protocol_turn_steer(
        &self,
        params: TurnSteerParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = params.expected_turn_id;
        self.runtime
            .steer_turn(
                params.thread_id,
                turn_id.clone(),
                protocol_turn_message(&params.input, params.prompt),
                protocol_turn_images(&params.input),
            )
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TurnSteerResult { turn_id }).unwrap())
    }

    async fn handle_team_start(
        &self,
        params: TeamStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .start_team(RuntimeTeamStartRequest {
                lead_thread_id: params.lead_thread_id,
                display_mode: params.display_mode.unwrap_or_default(),
                members: params
                    .members
                    .into_iter()
                    .map(|member| RuntimeTeamMemberStartRequest {
                        name: member.name,
                        model_provider: member.model_provider,
                        model: member.model,
                    })
                    .collect(),
            })
            .await
            .map_err(internal_error)?;
        let descriptor = team_descriptor(team);
        self.publish_notification(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "team/started".to_string(),
            params: serde_json::to_value(TeamStartedNotification {
                team: descriptor.clone(),
            })
            .unwrap(),
        });
        Ok(serde_json::to_value(TeamStartResult { team: descriptor }).unwrap())
    }

    async fn handle_team_list(
        &self,
        params: TeamListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut teams = self
            .runtime
            .list_teams()
            .await
            .into_iter()
            .map(team_descriptor)
            .collect::<Vec<_>>();
        if let Some(limit) = params.limit {
            teams.truncate(limit);
        }
        Ok(serde_json::to_value(TeamListResult {
            data: teams,
            next_cursor: None,
        })
        .unwrap())
    }

    async fn handle_team_read(
        &self,
        params: TeamReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(team) = self.runtime.read_team(&params.team_id).await else {
            return Ok(serde_json::to_value(TeamReadResult {
                team: None,
                messages: Vec::new(),
            })
            .unwrap());
        };
        let messages = team.mailbox.clone();
        Ok(serde_json::to_value(TeamReadResult {
            team: Some(team_descriptor(team)),
            messages,
        })
        .unwrap())
    }

    async fn handle_team_member_start(
        &self,
        params: TeamMemberStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .start_team_member(
                &params.team_id,
                RuntimeTeamMemberStartRequest {
                    name: params.name,
                    model_provider: params.model_provider,
                    model: params.model,
                },
            )
            .await
            .map_err(internal_error)?;
        let member = team
            .members
            .last()
            .cloned()
            .ok_or_else(|| internal_error("team member was not added"))?;
        Ok(serde_json::to_value(TeamMemberStartResult { member }).unwrap())
    }

    async fn handle_team_member_message(
        &self,
        params: TeamMemberMessageParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = self
            .runtime
            .message_team_member(&params.team_id, &params.member_id, params.text)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TeamMemberMessageResult { turn_id }).unwrap())
    }

    async fn handle_team_member_interrupt(
        &self,
        params: TeamMemberInterruptParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = self
            .runtime
            .interrupt_team_member(&params.team_id, &params.member_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TeamMemberInterruptResult {
            interrupted: turn_id.is_some(),
            turn_id,
        })
        .unwrap())
    }

    async fn handle_team_member_focus(
        &self,
        params: TeamMemberFocusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .read_team(&params.team_id)
            .await
            .ok_or_else(|| invalid_params(format!("unknown team {:?}", params.team_id)))?;
        if !team
            .members
            .iter()
            .any(|member| member.id == params.member_id)
        {
            return Err(invalid_params(format!(
                "unknown team member {:?}",
                params.member_id
            )));
        }
        Ok(serde_json::to_value(TeamMemberFocusResult {
            focused_member_id: params.member_id,
        })
        .unwrap())
    }

    async fn handle_team_cleanup(
        &self,
        params: TeamCleanupParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cleaned = self
            .runtime
            .cleanup_team(&params.team_id, params.force)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TeamCleanupResult { cleaned }).unwrap())
    }

    async fn protocol_thread_model(&self, thread_id: &str) -> Option<ProtocolThreadModelSelection> {
        if let Some(model) = self
            .protocol_thread_models
            .read()
            .await
            .get(thread_id)
            .cloned()
        {
            return Some(model);
        }
        self.runtime
            .list_threads()
            .await
            .ok()?
            .into_iter()
            .find(|metadata| metadata.thread_id == thread_id)
            .and_then(|metadata| match (metadata.provider, metadata.model) {
                (Some(provider), Some(model)) => Some(ProtocolThreadModelSelection {
                    provider,
                    model,
                    reasoning: None,
                }),
                _ => None,
            })
    }

    async fn handle_tools_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ToolsListResult {
            tools: self.runtime.tool_specs().await,
        })
        .unwrap())
    }

    async fn handle_tasks_submit(
        &self,
        params: TasksSubmitParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let task = self.submit_task(params).await?;
        Ok(serde_json::to_value(TasksSubmitResult { task }).unwrap())
    }

    async fn submit_task(
        &self,
        params: TasksSubmitParams,
    ) -> Result<roder_api::tasks::TaskHandle, JsonRpcError> {
        let runtime_cfg = self.runtime.status().await;
        let workspace = params.workspace.or(runtime_cfg.workspace).or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string())
        });
        let runner_destination = runtime_cfg.remote_runner_destination.clone();
        let runner_session = if let Some(destination) = runner_destination.clone() {
            let provider = self
                .runtime
                .registry()
                .remote_runner_providers
                .iter()
                .find(|provider| provider.id() == destination.provider_id)
                .cloned()
                .ok_or_else(|| {
                    internal_error(anyhow::anyhow!(
                        "remote runner provider {:?} is not installed",
                        destination.provider_id
                    ))
                })?;
            Some(
                provider
                    .create_session(destination)
                    .await
                    .map_err(internal_error)?,
            )
        } else {
            None
        };
        self.tasks
            .submit(
                params.executor_id,
                params.input,
                roder_tasks::TaskSubmitOptions {
                    thread_id: params.thread_id.clone(),
                    turn_id: params.turn_id,
                    workspace_root: workspace,
                    runner_destination,
                    runner_session,
                    ..roder_tasks::TaskSubmitOptions::default()
                },
            )
            .await
            .map_err(internal_error)
    }

    async fn handle_tasks_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(TasksListResult {
            tasks: self.tasks.list().await,
        })
        .unwrap())
    }

    async fn handle_tasks_get(
        &self,
        params: TasksGetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let task = self
            .tasks
            .get(&params.task_id)
            .await
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: format!("unknown task {:?}", params.task_id),
                data: None,
            })?;
        let (logs, dropped_bytes) =
            self.tasks
                .logs(&params.task_id)
                .await
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: format!("unknown task {:?}", params.task_id),
                    data: None,
                })?;
        Ok(serde_json::to_value(TasksGetResult {
            task,
            logs: logs
                .into_iter()
                .map(|entry| TaskLogDescriptor {
                    stream: entry.stream,
                    chunk: entry.chunk,
                    timestamp: entry.timestamp,
                })
                .collect(),
            dropped_bytes,
        })
        .unwrap())
    }

    async fn handle_tasks_cancel(
        &self,
        params: TasksCancelParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cancelled = self
            .tasks
            .cancel(&params.task_id, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TasksCancelResult { cancelled }).unwrap())
    }

    async fn handle_tasks_subscribe(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(TasksSubscribeResult {
            subscribed: true,
            event_kinds: vec![
                "task.started".to_string(),
                "task.output".to_string(),
                "task.completed".to_string(),
                "task.failed".to_string(),
                "task.cancelled".to_string(),
            ],
        })
        .unwrap())
    }

    async fn handle_webwright_prepare(
        &self,
        params: WebwrightPrepareParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace.clone())
            .await?;
        let result = crate::webwright::prepare(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_artifacts(
        &self,
        params: WebwrightWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let result = crate::webwright::artifacts(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_latest_run(
        &self,
        params: WebwrightWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let result = crate::webwright::latest_run(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_report(
        &self,
        params: WebwrightWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let result = crate::webwright::report(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_verify(
        &self,
        params: WebwrightWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let result = crate::webwright::verify(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_visual_judge(
        &self,
        params: WebwrightVisualJudgeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let cfg = self.runtime.status().await;
        let provider = cfg.default_provider;
        let model = cfg.default_model;
        let engine = self.runtime.registry().inference_engine(&provider);
        let provider_image_input = engine
            .as_ref()
            .map(|engine| engine.capabilities().image_input)
            .unwrap_or(false);
        let step = crate::webwright::visual_judge_step(
            &root,
            params,
            provider.clone(),
            model.clone(),
            provider_image_input,
        )
        .map_err(invalid_params)?;
        let result = match step {
            crate::webwright::WebwrightVisualJudgeStep::Done(result) => result,
            crate::webwright::WebwrightVisualJudgeStep::Ready {
                workspace,
                prepared,
                provider,
                model,
            } => match engine {
                Some(engine) => match self
                    .run_webwright_visual_judge(engine, &provider, &model, &prepared)
                    .await
                {
                    Ok(response) => crate::webwright::complete_visual_judge(
                        &prepared, provider, model, response,
                    )
                    .map_err(internal_error)?,
                    Err(err) => crate::webwright::fail_visual_judge(
                        &workspace,
                        Some(&prepared),
                        provider,
                        model,
                        err.to_string(),
                    )
                    .map_err(internal_error)?,
                },
                None => crate::webwright::fail_visual_judge(
                    &workspace,
                    Some(&prepared),
                    provider,
                    model,
                    "active provider is not registered".to_string(),
                )
                .map_err(internal_error)?,
            },
        };
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_export(
        &self,
        params: WebwrightExportParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let result = crate::webwright::export(&root, params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_setup(
        &self,
        params: WebwrightSetupParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let result = crate::webwright::setup(params).map_err(invalid_params)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_webwright_submit(
        &self,
        params: WebwrightSubmitParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let input = crate::webwright::submit_input(params.clone()).map_err(invalid_params)?;
        let task = self
            .submit_task(TasksSubmitParams {
                executor_id: crate::webwright::task_executor_id(),
                input,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                workspace: params.workspace,
            })
            .await?;
        Ok(serde_json::to_value(WebwrightSubmitResult { task }).unwrap())
    }

    async fn handle_webwright_rerun(
        &self,
        params: WebwrightRerunParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .webwright_workspace_root(params.workspace_root.clone())
            .await?;
        let thread_id = params.thread_id.clone();
        let turn_id = params.turn_id.clone();
        let plan = crate::webwright::prepare_rerun(&root, params).map_err(invalid_params)?;
        let task = self
            .submit_task(TasksSubmitParams {
                executor_id: crate::webwright::process_executor_id(),
                input: plan.task_input.clone(),
                thread_id,
                turn_id,
                workspace: Some(root.display().to_string()),
            })
            .await?;
        Ok(serde_json::to_value(crate::webwright::rerun_result(task, plan)).unwrap())
    }

    async fn webwright_workspace_root(
        &self,
        requested: Option<String>,
    ) -> Result<std::path::PathBuf, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        crate::webwright::workspace_root(runtime_workspace, requested).map_err(invalid_params)
    }

    async fn run_webwright_visual_judge(
        &self,
        engine: Arc<dyn InferenceEngine>,
        provider: &str,
        model: &str,
        prepared: &roder_ext_webwright::WebwrightPreparedVisualJudge,
    ) -> anyhow::Result<String> {
        let image_url = screenshot_data_url(&prepared.screenshot_path)?;
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::with_images(
                prepared.prompt.clone(),
                vec![InputImage { image_url }],
            ))],
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig {
                max_tokens: Some(600),
                temperature: Some(0.0),
                top_p: None,
                response_format: None,
            },
            runtime: RuntimeHints {
                profile: RuntimeProfile::Eval,
                hosted_web_search: HostedWebSearchConfig::disabled(),
                ..RuntimeHints::default()
            },
            metadata: serde_json::json!({
                "feature": "webwright_visual_judge",
                "workspaceStored": true
            }),
        };
        let thread_id = format!("webwright-visual-{}", uuid::Uuid::new_v4());
        let turn_id = "visual-judge";
        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: &thread_id,
                    turn_id,
                    tool_executor: None,
                },
                request,
            )
            .await?;
        let mut response = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                InferenceEvent::MessageDelta(delta) => response.push_str(&delta.text),
                InferenceEvent::Failed(failure) => anyhow::bail!(failure.message),
                InferenceEvent::Completed(_) => break,
                _ => {}
            }
        }
        Ok(response)
    }

    async fn handle_commands_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let registry = self.command_registry().await.map_err(internal_error)?;
        Ok(serde_json::to_value(CommandsListResult {
            commands: registry
                .iter()
                .map(|(_, spec)| command_descriptor(spec))
                .collect(),
        })
        .unwrap())
    }

    async fn handle_commands_expand(
        &self,
        params: CommandsExpandParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let expanded = self.expand_command(params).await?;
        Ok(serde_json::to_value(expanded).unwrap())
    }

    async fn handle_commands_run(
        &self,
        params: CommandsRunParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let thread_id = params.thread_id;
        let workspace = params.workspace.clone();
        let expanded = self
            .expand_command(CommandsExpandParams {
                name: params.name,
                arguments: params.arguments,
                workspace: params.workspace,
            })
            .await?;
        let workspace = match workspace {
            Some(workspace) => workspace,
            None => self
                .runtime
                .workspace_for_thread(&thread_id)
                .await
                .map_err(internal_error)?,
        };
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id,
                message: expanded.message.clone(),
                images: Vec::new(),
                provider_override: None,
                model_override: expanded.model.clone(),
                reasoning_override: None,
                workspace,
                instructions: default_instructions(),
                task_ledger_required: false,
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(CommandsRunResult { turn_id, expanded }).unwrap())
    }

    async fn expand_command(
        &self,
        params: CommandsExpandParams,
    ) -> Result<CommandsExpandResult, JsonRpcError> {
        let registry = self.command_registry().await.map_err(internal_error)?;
        let spec = registry.get(&params.name).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: format!("unknown command {:?}", params.name),
            data: None,
        })?;
        let cfg = roder_config::load_config()
            .map(|config| config.commands.unwrap_or_default())
            .map_err(internal_error)?;
        let runtime_cfg = self.runtime.status().await;
        let workspace = params
            .workspace
            .as_deref()
            .or(runtime_cfg.workspace.as_deref())
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "could not resolve command workspace".to_string(),
                data: None,
            })?;
        let skills = self.runtime.skills_snapshot().await;
        let expansion = expand_command(CommandExpansionRequest {
            spec,
            arguments: &params.arguments,
            workspace_root: &workspace,
            options: CommandExpansionOptions {
                allow_shell_includes: cfg.allow_shell_includes,
                allow_url_includes: cfg.allow_url_includes,
                allowed_url_hosts: cfg.allowed_url_hosts,
                include_timeout_seconds: cfg.include_timeout_seconds.unwrap_or(5),
                max_include_bytes: cfg.max_include_bytes.unwrap_or(65_536),
                policy_mode: runtime_cfg.policy_mode,
            },
            shell_runner: None,
            url_fetcher: None,
            skill_registry: Some(&skills),
        })
        .map_err(internal_error)?;
        Ok(CommandsExpandResult {
            command: command_descriptor(spec),
            message: expansion.message,
            context_blocks: expansion.context_blocks,
            allowed_tools: expansion.allowed_tools,
            model: expansion.model,
            agent: expansion.agent,
        })
    }

    async fn command_registry(&self) -> anyhow::Result<CommandsRegistry> {
        Ok(self
            .command_registry
            .get_or_try_init(|| async { self.build_command_registry().await })
            .await?
            .clone())
    }

    async fn build_command_registry(&self) -> anyhow::Result<CommandsRegistry> {
        let config = roder_config::load_config()?;
        let cfg = config.commands.clone().unwrap_or_default();
        if !cfg.enabled {
            anyhow::bail!("commands are disabled by configuration");
        }
        let workspace = self
            .runtime
            .status()
            .await
            .workspace
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok());
        let workspace_dir = cfg.workspace_dir.as_ref().map(|path| {
            if path.is_absolute() {
                path.clone()
            } else if let Some(workspace) = workspace.as_ref() {
                workspace.join(path)
            } else {
                path.clone()
            }
        });
        let user_dir = cfg
            .user_dir
            .as_deref()
            .map(expand_tilde)
            .or_else(|| Some(roder_config::config_dir().join("commands")));
        let workflow_dirs = roder_config::dynamic_workflows::resolve_workflow_directories(
            config.dynamic_workflows.as_ref(),
            workspace.as_deref(),
        );
        let user_workflow_dir = Some(workflow_dirs.user);
        let workspace_workflow_dir = Some(workflow_dirs.workspace);
        let command_dirs = command_registry_directories(user_dir, workspace_dir);
        let workflow_dirs =
            workflow_registry_directories(user_workflow_dir, workspace_workflow_dir);
        CommandsRegistry::from_directories_with_workflows(
            command_dirs,
            workflow_dirs,
            CommandsRegistryOptions {
                include_builtins: true,
                allow_builtin_override: false,
            },
        )
    }

    async fn handle_tool_call(
        &self,
        params: ToolCallParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if !matches!(
            params.tool_name.as_str(),
            "get_goal" | "create_goal" | "update_goal"
        ) {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("tool cannot be called directly: {}", params.tool_name),
                data: None,
            });
        }

        let result = self
            .runtime
            .execute_workflow_tool(params.thread_id, &params.tool_name, params.arguments)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ToolCallResult {
            text: result.text,
            data: result.data,
            is_error: result.is_error,
        })
        .unwrap())
    }

    async fn handle_agents_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(AgentsListResult {
            agents: self
                .runtime
                .subagent_definitions()
                .into_iter()
                .map(|definition| AgentDescriptor {
                    agent_type: definition.agent_type,
                    description: definition.description,
                    tools: definition.tools,
                    model: definition.model,
                    permission_mode: definition.permission_mode,
                    max_turns: definition.max_turns,
                    max_result_chars: definition.max_result_chars,
                })
                .collect(),
        })
        .unwrap())
    }

    async fn handle_subagent_traces_list(
        &self,
        params: SubagentTracesListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let Some(snapshot) = snapshot else {
            return Ok(
                serde_json::to_value(SubagentTracesListResult { traces: Vec::new() }).unwrap(),
            );
        };

        let mut order = Vec::new();
        let mut traces = std::collections::HashMap::new();
        for envelope in snapshot.events {
            match envelope.event {
                RoderEvent::SubagentTraceCreated(event)
                    if event.summary.parent.thread_id == params.thread_id
                        && event.summary.parent.turn_id == params.turn_id =>
                {
                    if !traces.contains_key(&event.summary.trace_id) {
                        order.push(event.summary.trace_id.clone());
                    }
                    traces.insert(event.summary.trace_id.clone(), event.summary);
                }
                RoderEvent::SubagentTraceCompleted(event)
                    if event.summary.parent.thread_id == params.thread_id
                        && event.summary.parent.turn_id == params.turn_id =>
                {
                    if !traces.contains_key(&event.summary.trace_id) {
                        order.push(event.summary.trace_id.clone());
                    }
                    traces.insert(event.summary.trace_id.clone(), event.summary);
                }
                RoderEvent::SubagentTraceFailed(event)
                    if event.summary.parent.thread_id == params.thread_id
                        && event.summary.parent.turn_id == params.turn_id =>
                {
                    if !traces.contains_key(&event.summary.trace_id) {
                        order.push(event.summary.trace_id.clone());
                    }
                    traces.insert(event.summary.trace_id.clone(), event.summary);
                }
                _ => {}
            }
        }

        let traces = order
            .into_iter()
            .filter_map(|trace_id| traces.remove(&trace_id))
            .collect();
        Ok(serde_json::to_value(SubagentTracesListResult { traces }).unwrap())
    }

    async fn handle_subagent_trace_read(
        &self,
        params: SubagentTraceReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let Some(snapshot) = snapshot else {
            return Ok(serde_json::to_value(SubagentTraceReadResult {
                trace_id: params.trace_id,
                events: Vec::new(),
                next_offset: None,
            })
            .unwrap());
        };
        let all_events = snapshot
            .events
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                RoderEvent::SubagentTraceDelta(event)
                    if event.delta.trace_id == params.trace_id =>
                {
                    Some(event.delta)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let offset = params.offset.min(all_events.len());
        let limit = params.limit.unwrap_or(100).max(1);
        let end = offset.saturating_add(limit).min(all_events.len());
        let next_offset = (end < all_events.len()).then_some(end);
        let events = all_events[offset..end].to_vec();

        Ok(serde_json::to_value(SubagentTraceReadResult {
            trace_id: params.trace_id,
            events,
            next_offset,
        })
        .unwrap())
    }

    async fn handle_plan_review_read(
        &self,
        params: PlanReviewReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let review = self
            .find_plan_review(&params.thread_id, &params.review_id)
            .await?;
        Ok(serde_json::to_value(PlanReviewReadResult { review }).unwrap())
    }

    async fn handle_plan_review_comment(
        &self,
        params: PlanReviewCommentParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(review) = self
            .find_plan_review(&params.thread_id, &params.review_id)
            .await?
        else {
            return Err(not_found(format!(
                "unknown plan review {:?}",
                params.review_id
            )));
        };
        let turn_id = review.turn_id.clone();
        let comment = PlanComment {
            id: uuid::Uuid::new_v4().to_string(),
            review_id: params.review_id,
            anchor: params.anchor,
            body: params.body,
            created_at: time::OffsetDateTime::now_utc(),
        };
        self.runtime
            .emit(RoderEvent::PlanReviewCommentAdded(
                roder_api::events::PlanReviewCommentAdded {
                    thread_id: params.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    review_id: comment.review_id.clone(),
                    comment: comment.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        let _ = self
            .runtime
            .steer_turn(
                params.thread_id.clone(),
                turn_id,
                format!("Plan review comment: {}", comment.body),
                Vec::new(),
            )
            .await;
        Ok(serde_json::to_value(PlanReviewCommentResult { comment }).unwrap())
    }

    async fn handle_plan_review_rewrite(
        &self,
        params: PlanReviewRewriteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(review) = self
            .find_plan_review(&params.thread_id, &params.review_id)
            .await?
        else {
            return Err(not_found(format!(
                "unknown plan review {:?}",
                params.review_id
            )));
        };
        let turn_id = review.turn_id.clone();
        let rewrite = PlanRewrite {
            id: uuid::Uuid::new_v4().to_string(),
            review_id: params.review_id,
            replacement_markdown: params.replacement_markdown,
            created_at: time::OffsetDateTime::now_utc(),
        };
        self.runtime
            .emit(RoderEvent::PlanReviewRewritten(
                roder_api::events::PlanReviewRewritten {
                    thread_id: params.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    review_id: rewrite.review_id.clone(),
                    rewrite: rewrite.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        let _ = self
            .runtime
            .steer_turn(
                params.thread_id.clone(),
                turn_id,
                format!("Plan rewrite requested:\n{}", rewrite.replacement_markdown),
                Vec::new(),
            )
            .await;
        Ok(serde_json::to_value(PlanReviewRewriteResult { rewrite }).unwrap())
    }

    async fn handle_plan_review_approve(
        &self,
        params: PlanReviewApproveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(review) = self
            .find_plan_review(&params.thread_id, &params.review_id)
            .await?
        else {
            return Err(not_found(format!(
                "unknown plan review {:?}",
                params.review_id
            )));
        };
        self.runtime
            .emit(RoderEvent::PlanReviewApproved(
                roder_api::events::PlanReviewApproved {
                    thread_id: params.thread_id,
                    turn_id: review.turn_id,
                    review_id: params.review_id,
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(PlanReviewApproveResult { approved: true }).unwrap())
    }

    async fn handle_plan_review_reject(
        &self,
        params: PlanReviewRejectParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(review) = self
            .find_plan_review(&params.thread_id, &params.review_id)
            .await?
        else {
            return Err(not_found(format!(
                "unknown plan review {:?}",
                params.review_id
            )));
        };
        self.runtime
            .emit(RoderEvent::PlanReviewRejected(
                roder_api::events::PlanReviewRejected {
                    thread_id: params.thread_id,
                    turn_id: review.turn_id,
                    review_id: params.review_id,
                    reason: params.reason,
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(PlanReviewRejectResult { rejected: true }).unwrap())
    }

    async fn handle_hunk_list(
        &self,
        params: HunkListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let hunks = self
            .load_hunks(&params.thread_id)
            .await?
            .into_iter()
            .filter(|hunk| {
                params
                    .turn_id
                    .as_ref()
                    .is_none_or(|turn_id| &hunk.turn_id == turn_id)
                    && params
                        .review_id
                        .as_ref()
                        .is_none_or(|review_id| hunk.plan_review_id.as_ref() == Some(review_id))
            })
            .collect();
        Ok(serde_json::to_value(HunkListResult { hunks }).unwrap())
    }

    async fn handle_hunk_read(
        &self,
        params: HunkReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let hunk = self
            .load_hunks(&params.thread_id)
            .await?
            .into_iter()
            .find(|hunk| hunk.id == params.hunk_id);
        let page = hunk.map(|hunk| {
            roder_api::plan_review::page_hunk_diff(
                hunk,
                params.offset,
                params.limit.unwrap_or(100).max(1),
            )
        });
        Ok(serde_json::to_value(HunkReadResult { page }).unwrap())
    }

    async fn handle_workspace_changes_list(
        &self,
        params: WorkspaceChangesListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let changes = self
            .load_workspace_changes(&params.thread_id)
            .await?
            .into_iter()
            .filter(|change| {
                params
                    .turn_id
                    .as_ref()
                    .is_none_or(|turn_id| &change.turn_id == turn_id)
            })
            .collect();
        Ok(serde_json::to_value(WorkspaceChangesListResult { changes }).unwrap())
    }

    async fn handle_workspace_list(
        &self,
        _params: WorkspaceListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        let result = self.workspaces.list(runtime_workspace).await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_workspace_create(
        &self,
        params: WorkspaceCreateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        let result = self.workspaces.create(runtime_workspace, params).await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_workspace_update(
        &self,
        params: WorkspaceUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        let result = self.workspaces.update(runtime_workspace, params).await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_workspace_forget(
        &self,
        params: WorkspaceForgetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        let result = self.workspaces.forget(runtime_workspace, params).await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_hunk_rollback(
        &self,
        params: HunkRollbackParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let Some(hunk) = self
            .load_hunks(&params.thread_id)
            .await?
            .into_iter()
            .find(|hunk| hunk.id == params.hunk_id)
        else {
            return Err(not_found(format!("unknown hunk {:?}", params.hunk_id)));
        };
        self.runtime
            .emit(RoderEvent::HunkRollbackRequested(
                roder_api::events::HunkRollbackRequested {
                    thread_id: params.thread_id.clone(),
                    turn_id: hunk.turn_id.clone(),
                    hunk_id: hunk.id.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        let error = if !params.confirmed {
            Some("rollback requires confirmation".to_string())
        } else if hunk.reverse_patch.is_none() {
            Some("rollback data is unavailable for this hunk".to_string())
        } else {
            self.apply_hunk_reverse_patch(&hunk).await.err()
        };
        self.runtime
            .emit(RoderEvent::HunkRollbackCompleted(
                roder_api::events::HunkRollbackCompleted {
                    thread_id: params.thread_id,
                    turn_id: hunk.turn_id,
                    hunk_id: hunk.id,
                    error: error.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(HunkRollbackResult {
            rolled_back: error.is_none(),
            error,
        })
        .unwrap())
    }

    async fn handle_vcs_status(
        &self,
        params: VcsWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::status(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_changes_list(
        &self,
        params: VcsChangesListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::list_changes(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_changes_read(
        &self,
        params: VcsChangesReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::read_change(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_select(
        &self,
        params: VcsSelectionParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.enforce_vcs_mutation_policy("vcs/select", &params)
            .await?;
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::select(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_snapshot_create(
        &self,
        params: VcsSnapshotCreateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.enforce_vcs_mutation_policy("vcs/snapshot/create", &params)
            .await?;
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::create_snapshot(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_restore(
        &self,
        params: VcsRestoreParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.enforce_vcs_mutation_policy("vcs/restore", &params)
            .await?;
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::restore(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_lines_list(
        &self,
        params: VcsWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::list_lines(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_lines_switch(
        &self,
        params: VcsLineSwitchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.enforce_vcs_mutation_policy("vcs/lines/switch", &params)
            .await?;
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::switch_line(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_vcs_sync(
        &self,
        params: VcsSyncParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.enforce_vcs_mutation_policy("vcs/sync", &params)
            .await?;
        let root = self
            .vcs_workspace_root(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let result = crate::vcs::sync(
            self.runtime.registry().version_control_resolver(),
            root.root.path,
            params,
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    async fn vcs_workspace_root(
        &self,
        workspace_id: &str,
        root_id: Option<&str>,
    ) -> Result<crate::workspaces::ResolvedWorkspaceRoot, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        self.workspaces
            .resolve_root(runtime_workspace, workspace_id, root_id)
            .await
    }

    async fn enforce_vcs_mutation_policy<T: serde::Serialize>(
        &self,
        method: &str,
        params: &T,
    ) -> Result<(), JsonRpcError> {
        let mode = self.runtime.status().await.policy_mode;
        let tool_call = ToolCall {
            id: format!("{method}-{}", uuid::Uuid::new_v4()),
            name: method.to_string(),
            arguments: serde_json::to_value(params).unwrap_or(serde_json::Value::Null),
            raw_arguments: serde_json::to_string(params).unwrap_or_default(),
            thread_id: "app-server".to_string(),
            turn_id: method.to_string(),
        };
        let ctx =
            ToolExecutionContext::new(tool_call.thread_id.clone(), tool_call.turn_id.clone(), mode);
        let decision = DefaultPolicyGate::new()
            .decide_with_contributors(
                &tool_call,
                mode,
                &ctx,
                &self.runtime.registry().policy_contributors,
            )
            .await
            .map_err(internal_error)?;
        match decision {
            PolicyDecision::Allowed | PolicyDecision::AutoApproved { .. } => Ok(()),
            PolicyDecision::Denied { reason } => Err(JsonRpcError {
                code: -32004,
                message: format!("{method} denied by policy: {reason}"),
                data: Some(serde_json::json!({ "kind": "policy_denied" })),
            }),
            PolicyDecision::RequiresApproval { reason } => {
                let approved = self
                    .runtime
                    .request_app_server_tool_approval(tool_call, reason)
                    .await
                    .map_err(internal_error)?;
                if approved {
                    Ok(())
                } else {
                    Err(JsonRpcError {
                        code: -32004,
                        message: format!("{method} approval denied"),
                        data: Some(serde_json::json!({ "kind": "approval_denied" })),
                    })
                }
            }
        }
    }

    async fn handle_workflow_scan(
        &self,
        params: WorkflowScanParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let scan = self
            .workflow_scan(params.workspace, params.include_user)
            .await?;
        self.runtime
            .emit(RoderEvent::WorkflowImportsDetected(
                roder_api::events::WorkflowImportsDetected {
                    workspace: scan.workspace.clone(),
                    items: scan.items.clone(),
                    errors: scan.errors.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(WorkflowScanResult { scan }).unwrap())
    }

    async fn handle_workflow_preview(
        &self,
        params: WorkflowPreviewParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut items = self.workflow_scan(params.workspace, true).await?.items;
        if let Some(item_id) = params.item_id {
            items.retain(|item| item.id == item_id);
        }
        for item in &mut items {
            item.state = WorkflowImportState::Previewed;
            self.runtime
                .emit(RoderEvent::WorkflowImportPreviewed(
                    roder_api::events::WorkflowImportPreviewed {
                        item: item.clone(),
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }
        Ok(serde_json::to_value(WorkflowPreviewResult { items }).unwrap())
    }

    async fn handle_workflow_enable(
        &self,
        params: WorkflowEnableParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut item = self
            .find_workflow_item(params.workspace, &params.item_id)
            .await?;
        if item.approval_required && !params.approve_side_effects {
            return Err(JsonRpcError {
                code: -32040,
                message: format!("workflow import {:?} requires approval", item.id),
                data: Some(serde_json::json!({
                    "itemId": item.id,
                    "source": item.source,
                    "risk": item.risk,
                })),
            });
        }
        item.state = WorkflowImportState::Enabled;
        item.enabled_at = Some(time::OffsetDateTime::now_utc());
        let decision = self
            .record_workflow_decision(
                &item,
                WorkflowImportDecisionKind::Enable,
                params.approve_side_effects,
            )
            .await?;
        self.runtime
            .emit(RoderEvent::WorkflowImportEnabled(
                roder_api::events::WorkflowImportEnabled {
                    item: item.clone(),
                    decision: decision.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(WorkflowEnableResult { item, decision }).unwrap())
    }

    async fn handle_workflow_ignore(
        &self,
        params: WorkflowIgnoreParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let item = self
            .find_workflow_item(params.workspace, &params.item_id)
            .await?;
        let decision = self
            .record_workflow_decision(&item, WorkflowImportDecisionKind::Ignore, false)
            .await?;
        Ok(serde_json::to_value(WorkflowIgnoreResult {
            item_id: item.id,
            decision,
        })
        .unwrap())
    }

    async fn handle_workflow_refresh(
        &self,
        params: WorkflowRefreshParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut scan = self.workflow_scan(params.workspace, true).await?;
        let decisions = load_workflow_decisions().map_err(internal_error)?;
        let mut stale = Vec::new();
        for item in &mut scan.items {
            if let Some(decision) = decisions.iter().find(|decision| {
                decision.item_id == item.id
                    && matches!(decision.decision, WorkflowImportDecisionKind::Enable)
            }) && decision.source_hash != item.source.hash
            {
                item.state = WorkflowImportState::Stale;
                stale.push(item.clone());
                self.runtime
                    .emit(RoderEvent::WorkflowImportStale(
                        roder_api::events::WorkflowImportStale {
                            item: item.clone(),
                            previous_hash: decision.source_hash.clone(),
                            timestamp: time::OffsetDateTime::now_utc(),
                        },
                    ))
                    .await;
            }
        }
        Ok(serde_json::to_value(WorkflowRefreshResult { scan, stale }).unwrap())
    }

    async fn handle_workflow_remove(
        &self,
        params: WorkflowRemoveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let item = self
            .find_workflow_item(params.workspace, &params.item_id)
            .await?;
        let decision = self
            .record_workflow_decision(&item, WorkflowImportDecisionKind::Remove, false)
            .await?;
        self.runtime
            .emit(RoderEvent::WorkflowImportDisabled(
                roder_api::events::WorkflowImportDisabled {
                    item_id: item.id.clone(),
                    decision: decision.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(WorkflowRemoveResult {
            item_id: item.id,
            state: WorkflowImportState::Removed,
            decision,
        })
        .unwrap())
    }

    async fn handle_workflows_plan(
        &self,
        params: WorkflowsPlanParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .plan(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_approve(
        &self,
        params: WorkflowsApproveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .approve(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_list(
        &self,
        params: WorkflowsListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .list(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_get(
        &self,
        params: WorkflowsGetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .get(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_pause(
        &self,
        params: WorkflowsPauseParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .pause(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_resume(
        &self,
        params: WorkflowsResumeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .resume(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_stop(
        &self,
        params: WorkflowsStopParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .stop(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_restart_agent(
        &self,
        params: WorkflowsRestartAgentParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .restart_agent(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_save(
        &self,
        params: WorkflowsSaveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .save(params)
            .await
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_scripts_list(
        &self,
        params: WorkflowsScriptsListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .scripts_list(params)
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_scripts_read(
        &self,
        params: WorkflowsScriptsReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .scripts_read(params)
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn handle_workflows_scripts_delete(
        &self,
        params: WorkflowsScriptsDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.workflows
            .scripts_delete(params)
            .map(|result| serde_json::to_value(result).unwrap())
            .map_err(internal_error)
    }

    async fn workflow_scan(
        &self,
        workspace: Option<String>,
        include_user: bool,
    ) -> Result<WorkflowImportScan, JsonRpcError> {
        let workspace = match workspace {
            Some(workspace) => std::path::PathBuf::from(workspace),
            None => self
                .runtime
                .status()
                .await
                .workspace
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| JsonRpcError {
                    code: -32000,
                    message: "could not resolve workflow import workspace".to_string(),
                    data: None,
                })?,
        };
        let mut options = roder_config::WorkflowScanOptions::new(workspace);
        options.include_user = include_user;
        if include_user {
            options.user_roots.push(roder_config::config_dir());
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                options.user_roots.push(home.join(".agents"));
            }
        }
        Ok(roder_config::scan_workflow_imports(options))
    }

    async fn find_workflow_item(
        &self,
        workspace: Option<String>,
        item_id: &str,
    ) -> Result<WorkflowImportItem, JsonRpcError> {
        self.workflow_scan(workspace, true)
            .await?
            .items
            .into_iter()
            .find(|item| item.id == item_id)
            .ok_or_else(|| not_found(format!("unknown workflow import {item_id:?}")))
    }

    async fn record_workflow_decision(
        &self,
        item: &WorkflowImportItem,
        decision: WorkflowImportDecisionKind,
        approved_side_effects: bool,
    ) -> Result<WorkflowImportDecision, JsonRpcError> {
        let decision = WorkflowImportDecision {
            item_id: item.id.clone(),
            decision,
            source_hash: item.source.hash.clone(),
            approved_side_effects,
            decided_at: time::OffsetDateTime::now_utc(),
        };
        let mut decisions = load_workflow_decisions().map_err(internal_error)?;
        decisions.retain(|existing| existing.item_id != decision.item_id);
        decisions.push(decision.clone());
        save_workflow_decisions(&decisions).map_err(internal_error)?;
        Ok(decision)
    }

    async fn handle_media_list(
        &self,
        params: MediaListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut artifacts = self.media_store()?.list().map_err(internal_error)?;
        if let Some(kind) = params.kind {
            artifacts.retain(|artifact| artifact.kind == kind);
        }
        Ok(serde_json::to_value(MediaListResult { artifacts }).unwrap())
    }

    async fn handle_media_read(
        &self,
        params: MediaReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (artifact, bytes) = self
            .media_store()?
            .read(&params.artifact_id, params.max_bytes)
            .map_err(internal_error)?;
        Ok(serde_json::to_value(MediaReadResult {
            artifact,
            bytes_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        })
        .unwrap())
    }

    async fn handle_media_thumbnail(
        &self,
        params: MediaThumbnailParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let preview = self
            .media_store()?
            .preview(&params.artifact_id)
            .map_err(internal_error)?;
        Ok(serde_json::to_value(MediaThumbnailResult { preview }).unwrap())
    }

    async fn handle_media_delete(
        &self,
        params: MediaDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let deleted = self
            .media_store()?
            .delete(&params.artifact_id)
            .map_err(internal_error)?;
        if deleted {
            self.runtime
                .emit(RoderEvent::MediaArtifactDeleted(
                    roder_api::events::MediaArtifactDeleted {
                        artifact_id: params.artifact_id,
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }
        Ok(serde_json::to_value(MediaDeleteResult { deleted }).unwrap())
    }

    async fn handle_media_attach_to_turn(
        &self,
        params: MediaAttachToTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (artifact, bytes) = self
            .media_store()?
            .read(&params.artifact_id, None)
            .map_err(internal_error)?;
        let bytes_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let attachment = MediaAttachment {
            artifact_id: artifact.id.clone(),
            mime_type: artifact.mime_type.clone(),
            data_url: data_url(&artifact.mime_type, &bytes_base64),
        };
        let image = artifact_is_image(&artifact).then(|| roder_api::transcript::InputImage {
            image_url: attachment.data_url.clone(),
        });
        Ok(serde_json::to_value(MediaAttachToTurnResult { attachment, image }).unwrap())
    }

    fn media_store(&self) -> Result<MediaArtifactStore, JsonRpcError> {
        let cfg = roder_config::load_config()
            .unwrap_or_default()
            .media
            .unwrap_or_default();
        let root = cfg
            .artifacts_dir
            .or_else(|| std::env::var_os("RODER_MEDIA_ARTIFACT_DIR").map(std::path::PathBuf::from))
            .map(Ok)
            .unwrap_or_else(default_media_artifact_dir)
            .map_err(internal_error)?;
        Ok(MediaArtifactStore::new(root)
            .with_max_read_bytes(cfg.max_read_bytes.unwrap_or(10 * 1024 * 1024)))
    }

    async fn handle_memory_list(
        &self,
        params: MemoryListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let memories = self
            .memory_store()?
            .list(params.scope, params.limit.unwrap_or(50))
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(MemoryListResult { memories }).unwrap())
    }

    async fn handle_memory_read(
        &self,
        params: MemoryReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let memory = self
            .memory_store()?
            .get(&params.memory_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(MemoryReadResult { memory }).unwrap())
    }

    async fn handle_memory_save(
        &self,
        params: MemorySaveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let now = time::OffsetDateTime::now_utc();
        let record = MemoryRecord {
            id: None,
            scope: params.scope,
            text: params.text,
            content_hash: None,
            metadata: params.metadata,
            usage: None,
            deleted: false,
            created_at: now,
            updated_at: now,
        };
        let store = self.memory_store()?;
        let memory_id = store.put(record).await.map_err(internal_error)?;
        if let Some(memory) = store.get(&memory_id).await.map_err(internal_error)? {
            self.runtime
                .emit(RoderEvent::MemorySaved(roder_api::events::MemorySaved {
                    memory,
                    timestamp: time::OffsetDateTime::now_utc(),
                }))
                .await;
        }
        Ok(serde_json::to_value(MemorySaveResult { memory_id }).unwrap())
    }

    async fn handle_memory_update(
        &self,
        params: MemoryUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = self.memory_store()?;
        let existing = store
            .get(&params.memory_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| not_found(format!("unknown memory {:?}", params.memory_id)))?;
        let record = MemoryRecord {
            id: Some(params.memory_id.clone()),
            scope: existing.scope,
            text: params.text,
            content_hash: None,
            metadata: params.metadata,
            usage: existing.usage,
            deleted: false,
            created_at: existing.created_at,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        let memory_id = store.put(record).await.map_err(internal_error)?;
        if let Some(memory) = store.get(&memory_id).await.map_err(internal_error)? {
            self.runtime
                .emit(RoderEvent::MemoryUpdated(
                    roder_api::events::MemoryUpdated {
                        memory,
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }
        Ok(serde_json::to_value(MemorySaveResult { memory_id }).unwrap())
    }

    async fn handle_memory_delete(
        &self,
        params: MemoryDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let existed = self
            .memory_store()?
            .get(&params.memory_id)
            .await
            .map_err(internal_error)?
            .is_some();
        if existed {
            self.memory_store()?
                .delete(&params.memory_id)
                .await
                .map_err(internal_error)?;
            self.runtime
                .emit(RoderEvent::MemoryDeleted(
                    roder_api::events::MemoryDeleted {
                        memory_id: params.memory_id.clone(),
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }
        Ok(serde_json::to_value(MemoryDeleteResult { deleted: existed }).unwrap())
    }

    async fn handle_memory_query(
        &self,
        params: MemoryQueryParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let results = self
            .memory_store()?
            .search(MemoryQuery {
                scope: params.scope.clone(),
                text: params.text.clone(),
                limit: params.limit.unwrap_or(10),
                include_global: params.include_global,
                provider_id: None,
                model: None,
            })
            .await
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::MemoryQueried(
                roder_api::events::MemoryQueried {
                    scope: params.scope,
                    query: params.text,
                    result_count: results.len(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(MemoryQueryResult { results }).unwrap())
    }

    async fn handle_memory_provider_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let providers = self
            .runtime
            .registry()
            .embedding_providers
            .iter()
            .map(|provider| provider.descriptor())
            .collect::<Vec<_>>();
        Ok(serde_json::to_value(MemoryProviderListResult {
            providers,
            selected: selected_memory_provider(),
        })
        .unwrap())
    }

    async fn handle_memory_provider_set(
        &self,
        params: MemoryProviderSetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let selected = MemoryProviderSelection {
            provider_id: params.provider_id,
            model: params.model,
        };
        roder_config::save_memory_embedding_provider(&selected.provider_id, &selected.model)
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::MemoryProviderChanged(
                roder_api::events::MemoryProviderChanged {
                    provider: selected.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(selected).unwrap())
    }

    async fn handle_memory_recall_preview(
        &self,
        params: MemoryRecallPreviewParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let results = self
            .memory_store()?
            .search(MemoryQuery {
                scope: params.scope,
                text: params.text,
                limit: params.limit.unwrap_or(5),
                include_global: params.include_global,
                provider_id: None,
                model: None,
            })
            .await
            .map_err(internal_error)?;
        let citations = results
            .iter()
            .filter_map(|result| result.citation.clone())
            .collect::<Vec<_>>();
        self.runtime
            .emit(RoderEvent::MemoryRecallReady(
                roder_api::events::MemoryRecallReady {
                    thread_id: params.thread_id,
                    turn_id: params.turn_id,
                    citations: citations.clone(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(MemoryRecallPreviewResult { citations, results }).unwrap())
    }

    fn memory_store(&self) -> Result<Arc<dyn roder_api::memory::MemoryStore>, JsonRpcError> {
        self.runtime
            .registry()
            .memory_stores
            .first()
            .map(|factory| factory.create())
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "No memory store is registered".to_string(),
                data: None,
            })
    }

    async fn find_plan_review(
        &self,
        thread_id: &String,
        review_id: &str,
    ) -> Result<Option<PlanReview>, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(thread_id)
            .await
            .map_err(internal_error)?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let mut review = None;
        for envelope in snapshot.events {
            match envelope.event {
                RoderEvent::PlanReviewCreated(event) if event.review.id == review_id => {
                    review = Some(event.review);
                }
                RoderEvent::PlanReviewStatusChanged(event) if event.review_id == review_id => {
                    if let Some(review) = review.as_mut() {
                        review.status = event.status;
                        review.updated_at = event.timestamp;
                    }
                }
                RoderEvent::PlanReviewCommentAdded(event) if event.review_id == review_id => {
                    if let Some(review) = review.as_mut() {
                        review.comments.push(event.comment);
                        review.updated_at = event.timestamp;
                    }
                }
                RoderEvent::PlanReviewRewritten(event) if event.review_id == review_id => {
                    if let Some(review) = review.as_mut() {
                        review.status = PlanReviewStatus::Rewritten;
                        review.markdown = event.rewrite.replacement_markdown.clone();
                        review.rewrites.push(event.rewrite);
                        review.updated_at = event.timestamp;
                    }
                }
                RoderEvent::PlanReviewApproved(event) if event.review_id == review_id => {
                    if let Some(review) = review.as_mut() {
                        review.status = PlanReviewStatus::Approved;
                        review.updated_at = event.timestamp;
                    }
                }
                RoderEvent::PlanReviewRejected(event) if event.review_id == review_id => {
                    if let Some(review) = review.as_mut() {
                        review.status = PlanReviewStatus::Rejected;
                        review.updated_at = event.timestamp;
                    }
                }
                _ => {}
            }
        }
        Ok(review)
    }

    async fn load_hunks(&self, thread_id: &String) -> Result<Vec<HunkRecord>, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(thread_id)
            .await
            .map_err(internal_error)?;
        let Some(snapshot) = snapshot else {
            return Ok(Vec::new());
        };
        Ok(snapshot
            .events
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                RoderEvent::HunkRecorded(event) => Some(event.hunk),
                _ => None,
            })
            .collect())
    }

    async fn load_workspace_changes(
        &self,
        thread_id: &String,
    ) -> Result<Vec<roder_api::workspace_changes::WorkspaceChangeObservation>, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(thread_id)
            .await
            .map_err(internal_error)?;
        let Some(snapshot) = snapshot else {
            return Ok(Vec::new());
        };
        Ok(snapshot
            .events
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                RoderEvent::WorkspaceChangeObserved(event) => Some(event.change),
                _ => None,
            })
            .collect())
    }

    async fn apply_hunk_reverse_patch(&self, hunk: &HunkRecord) -> Result<(), String> {
        let workspace = self
            .runtime
            .status()
            .await
            .workspace
            .ok_or_else(|| "rollback requires a configured workspace".to_string())?;
        let path = safe_workspace_path(std::path::Path::new(&workspace), &hunk.path)?;
        let text =
            std::fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", hunk.path))?;
        let old_text = hunk
            .diff
            .iter()
            .filter(|line| matches!(line.kind, roder_api::plan_review::HunkDiffLineKind::Removed))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let new_text = hunk
            .diff
            .iter()
            .filter(|line| matches!(line.kind, roder_api::plan_review::HunkDiffLineKind::Added))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if new_text.is_empty() {
            return Err("rollback cannot infer changed text for this hunk".to_string());
        }
        let Some(index) = text.find(&new_text) else {
            return Err(format!(
                "rollback conflict: expected changed text not found in {}",
                hunk.path
            ));
        };
        let mut updated = text;
        updated.replace_range(index..index + new_text.len(), &old_text);
        std::fs::write(&path, updated).map_err(|err| format!("write {}: {err}", hunk.path))?;
        Ok(())
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.runtime.subscribe_events()
    }

    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.protocol_notifications.subscribe()
    }

    pub(crate) fn publish_notification(&self, notification: JsonRpcNotification) {
        let _ = self.protocol_notifications.send(notification);
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn shell_settings(shell: &str) -> ShellSettings {
    ShellSettings {
        shell: shell.to_string(),
        options: roder_api::command_shell::command_shell_options(shell),
    }
}

pub(crate) fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

fn active_model_profile_label(cfg: &roder_core::RuntimeConfig) -> String {
    cfg.model_profiles
        .get(&cfg.default_model)
        .cloned()
        .or_else(|| built_in_model_profile(&cfg.default_model))
        .map(|profile| profile.model)
        .unwrap_or_else(|| cfg.default_model.clone())
}

fn not_found(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

fn sanitize_roadmap_slug(slug: &str) -> Result<String, JsonRpcError> {
    if slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        Ok(slug.to_string())
    } else {
        Err(invalid_params(
            "slug must contain only lowercase letters, digits, and hyphens",
        ))
    }
}

fn next_roadmap_phase(roadmap_dir: &std::path::Path) -> anyhow::Result<u32> {
    let mut max_phase = 0;
    for entry in std::fs::read_dir(roadmap_dir)? {
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

fn resolve_roadmap_path(
    workspace: &std::path::Path,
    path: &str,
) -> anyhow::Result<std::path::PathBuf> {
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

fn rel(workspace: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn team_descriptor(team: TeamState) -> TeamDescriptor {
    TeamDescriptor {
        id: team.id,
        lead_thread_id: team.lead_thread_id,
        display_mode: team.display_mode,
        members: team.members,
        tasks: team.tasks,
    }
}

fn invalid_params_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32602,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

fn split_pane_unsupported_error(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32601,
        message: format!(
            "{method} is only available inside a split-pane TUI backend; headless app-server clients should use team/member/focus"
        ),
        data: Some(serde_json::json!({
            "supportedAlternative": "team/member/focus"
        })),
    }
}

fn runner_status(
    destination: Option<&roder_api::remote_runner::RunnerDestination>,
    session_id: Option<String>,
) -> Option<RunnerStatus> {
    destination.map(|destination| RunnerStatus {
        destination_id: destination.id.clone(),
        provider_id: destination.provider_id.clone(),
        state: if session_id.is_some() {
            "active".to_string()
        } else {
            "configured".to_string()
        },
        session_id,
    })
}

fn web_search_mode_config_value(mode: HostedWebSearchMode) -> &'static str {
    match mode {
        HostedWebSearchMode::Disabled => "disabled",
        HostedWebSearchMode::Cached => "cached",
        HostedWebSearchMode::Live => "live",
    }
}

fn policy_mode_config_value(mode: roder_api::policy_mode::PolicyMode) -> &'static str {
    match mode {
        roder_api::policy_mode::PolicyMode::Default => "default",
        roder_api::policy_mode::PolicyMode::AcceptAll => "accept_edits",
        roder_api::policy_mode::PolicyMode::Plan => "plan",
        roder_api::policy_mode::PolicyMode::Bypass => "bypass",
    }
}

fn command_descriptor(spec: &CommandSpec) -> CommandDescriptor {
    CommandDescriptor {
        name: spec.name.clone(),
        description: spec.description.clone(),
        argument_hint: spec.argument_hint.clone(),
        source: spec.display_source(),
        model: spec.model.clone(),
        agent: spec.agent.clone(),
        has_shell_includes: !spec.include.shell.is_empty(),
        has_url_includes: !spec.include.urls.is_empty(),
    }
}

fn command_registry_directories(
    user_dir: Option<std::path::PathBuf>,
    workspace_dir: Option<std::path::PathBuf>,
) -> Vec<CommandDirectory> {
    let mut directories = Vec::new();
    if let Some(root) = user_dir {
        directories.push(CommandDirectory {
            root,
            source: CommandSource::User,
        });
    }
    if let Some(root) = workspace_dir {
        directories.push(CommandDirectory {
            root,
            source: CommandSource::Workspace,
        });
    }
    directories
}

fn workflow_registry_directories(
    user_dir: Option<std::path::PathBuf>,
    workspace_dir: Option<std::path::PathBuf>,
) -> Vec<WorkflowCommandDirectory> {
    let mut directories = Vec::new();
    if let Some(root) = user_dir {
        directories.push(WorkflowCommandDirectory {
            root,
            source: CommandSource::User,
        });
    }
    if let Some(root) = workspace_dir {
        directories.push(WorkflowCommandDirectory {
            root,
            source: CommandSource::Workspace,
        });
    }
    directories
}

fn expand_tilde(path: &std::path::Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn safe_workspace_path(
    workspace: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("rollback path escapes workspace: {relative}"));
    }
    Ok(workspace.join(path))
}

fn workflow_decisions_path() -> anyhow::Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("RODER_WORKFLOW_IMPORTS_PATH") {
        return Ok(std::path::PathBuf::from(path));
    }
    Ok(roder_config::config_dir().join("workflow-imports.json"))
}

fn load_workflow_decisions() -> anyhow::Result<Vec<WorkflowImportDecision>> {
    let path = workflow_decisions_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text)?)
}

fn save_workflow_decisions(decisions: &[WorkflowImportDecision]) -> anyhow::Result<()> {
    let path = workflow_decisions_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(decisions)?;
    std::fs::write(path, text)?;
    Ok(())
}

fn artifact_is_image(artifact: &MediaArtifact) -> bool {
    matches!(artifact.kind, roder_api::media::MediaKind::Image)
        && artifact.mime_type.starts_with("image/")
}

fn screenshot_data_url(path: &std::path::Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("read screenshot {}", path.display()))?;
    let bytes_base64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    Ok(data_url("image/png", &bytes_base64))
}

fn selected_memory_provider() -> MemoryProviderSelection {
    let memories = roder_config::load_config()
        .unwrap_or_default()
        .memories
        .unwrap_or_default();
    MemoryProviderSelection {
        provider_id: memories
            .embedding_provider
            .unwrap_or_else(|| "openai".to_string()),
        model: memories
            .embedding_model
            .unwrap_or_else(|| "text-embedding-3-large".to_string()),
    }
}

async fn provider_auth_status(
    provider_id: &str,
    metadata: &InferenceProviderMetadata,
) -> (bool, Option<String>) {
    match metadata.auth_type {
        ProviderAuthType::None => (true, None),
        ProviderAuthType::ApiKey => {
            let configured = if let Ok(cfg) = roder_config::load_config() {
                if let Some(p) = cfg.providers.get(provider_id) {
                    p.api_key.is_some()
                        || p.api_key_env
                            .as_ref()
                            .is_some_and(|var| std::env::var(var).is_ok())
                } else {
                    metadata.auth_configured.unwrap_or(true)
                }
            } else {
                metadata.auth_configured.unwrap_or(true)
            };
            (configured, metadata.auth_label.clone())
        }
        ProviderAuthType::OAuth if provider_id == roder_api::catalog::PROVIDER_CODEX => {
            match roder_codex_auth::status().await {
                Ok(Some(tokens)) if !tokens.account_id.is_empty() => {
                    (true, Some(tokens.account_id))
                }
                Ok(Some(_)) => (true, None),
                Ok(None) | Err(_) => (false, None),
            }
        }
        ProviderAuthType::OAuth if provider_id == roder_api::catalog::PROVIDER_SUPERGROK => {
            match roder_supergrok_auth::status().await {
                Ok(Some(tokens)) if !tokens.email.is_empty() => (true, Some(tokens.email)),
                Ok(Some(_)) => (true, None),
                Ok(None) | Err(_) => (false, None),
            }
        }
        ProviderAuthType::OAuth => (false, metadata.auth_label.clone()),
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::catalog::PROVIDER_MOCK;
    use roder_api::inference::{
        ModelHarnessProfile, ModelInstructionOverlay, ModelProfileReasoning, ModelSchemaPolicy,
        ProviderFamily,
    };

    #[test]
    fn model_switch_provider_status_fields_serialize_with_active_profile() {
        let cfg = roder_core::RuntimeConfig {
            default_provider: PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.5".to_string(),
            model_profiles: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                ModelHarnessProfile {
                    model: "gpt-5.5".to_string(),
                    provider: PROVIDER_MOCK.to_string(),
                    provider_family: ProviderFamily::Mock,
                    edit_tool: Some("patch".to_string()),
                    schema_policy: ModelSchemaPolicy::RequiredFirstFlat,
                    instruction_overlay: ModelInstructionOverlay::LiteralToolOutputs,
                    reasoning: ModelProfileReasoning::default(),
                    parallel_tool_calls: Some(true),
                    auto_compact_token_limit: Some(180_000),
                },
            )]),
            ..roder_core::RuntimeConfig::default()
        };
        let result = ProviderSelectResult {
            provider: cfg.default_provider.clone(),
            model: cfg.default_model.clone(),
            reasoning: "medium".to_string(),
            model_profile: Some(active_model_profile_label(&cfg)),
            model_switch_summary: Some(
                "Model switch summary: previous profile mock/mock. Current profile mock/gpt-5.5."
                    .to_string(),
            ),
        };

        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["modelProfile"], "gpt-5.5");
        assert!(
            value["modelSwitchSummary"]
                .as_str()
                .unwrap()
                .contains("previous profile mock/mock")
        );
    }
}
