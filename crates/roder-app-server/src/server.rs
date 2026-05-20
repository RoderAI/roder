use std::sync::Arc;

use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::inference::{
    HostedWebSearchMode, InferenceProviderContext, InferenceProviderMetadata, ProviderAuthType,
};
use roder_api::media::{MediaArtifact, MediaAttachment, data_url};
use roder_api::memory::{MemoryProviderSelection, MemoryQuery, MemoryRecord};
use roder_api::plan_review::{HunkRecord, PlanComment, PlanReview, PlanReviewStatus, PlanRewrite};
use roder_api::workflow::{
    WorkflowImportDecision, WorkflowImportDecisionKind, WorkflowImportItem, WorkflowImportScan,
    WorkflowImportState,
};
use roder_commands::{
    CommandExpansionOptions, CommandExpansionRequest, CommandSpec, CommandsRegistry,
    CommandsRegistryOptions, ExtensionCommandDirectory, expand_command,
};
use roder_core::{
    Runtime, StartTurnRequest, TeamMemberStartRequest as RuntimeTeamMemberStartRequest,
    TeamStartRequest as RuntimeTeamStartRequest, TeamState, default_instructions,
    media_artifacts::{MediaArtifactStore, default_media_artifact_dir},
};
use roder_protocol::*;
use roder_tasks::{BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry};
use time::OffsetDateTime;
use tokio::sync::{RwLock, broadcast};

use crate::desktop_contract::{
    default_cwd_string, desktop_thread_from_metadata, desktop_turn_from_record,
    desktop_turn_images, desktop_turn_message,
};
use crate::notifications;

pub struct AppServer {
    pub runtime: Arc<Runtime>,
    tasks: BackgroundRunner,
    persist_user_config: bool,
    desktop_threads: RwLock<std::collections::HashMap<String, DesktopThread>>,
    desktop_thread_models: RwLock<std::collections::HashMap<String, (String, String)>>,
    desktop_active_turns: RwLock<std::collections::HashMap<String, String>>,
    desktop_notifications: broadcast::Sender<JsonRpcNotification>,
}

impl AppServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let mut task_registry = TaskExecutorRegistry::default();
        for executor in &runtime.registry().task_executors {
            let _ = task_registry.register(Arc::clone(executor));
        }
        let tasks = BackgroundRunner::new(task_registry, BackgroundRunnerConfig::default());
        let (desktop_notifications, _) = broadcast::channel(1024);
        if tokio::runtime::Handle::try_current().is_ok() {
            notifications::spawn_task_event_bridge(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_runtime_event_handlers(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_desktop_notification_bridge(
                Arc::clone(&runtime),
                desktop_notifications.clone(),
            );
        }
        Self {
            runtime,
            tasks,
            persist_user_config: false,
            desktop_threads: RwLock::new(std::collections::HashMap::new()),
            desktop_thread_models: RwLock::new(std::collections::HashMap::new()),
            desktop_active_turns: RwLock::new(std::collections::HashMap::new()),
            desktop_notifications,
        }
    }

    pub fn with_user_config_persistence(mut self) -> Self {
        self.persist_user_config = true;
        self
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let result = match req.method.as_str() {
            "initialize" => self.handle_initialize().await,
            "extensions/list" => self.handle_extensions_list().await,
            "providers/list" => self.handle_providers_list().await,
            "providers/configure" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_provider_configure(p).await
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
            "settings/set_default_mode" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_settings_set_default_mode(p).await
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
            "thread/archive" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_thread_archive(p).await
                })
                .await
            }
            "session/get" => self.handle_session_get().await,
            "session/set_mode" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_session_set_mode(p).await
                })
                .await
            }
            "session/exit_plan" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_session_exit_plan(p).await
                })
                .await
            }
            "session/resolve_approval" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_session_resolve_approval(p).await
                })
                .await
            }
            "session/resolve_user_input" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_session_resolve_user_input(p).await
                })
                .await
            }
            "turn/start" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_desktop_turn_start(p).await
                })
                .await
            }
            "turn/interrupt" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_desktop_turn_interrupt(p).await
                })
                .await
            }
            "turn/steer" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_desktop_turn_steer(p).await
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
            "command/exec" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_command_exec(p).await },
                )
                .await
            }
            "tools/list" => self.handle_tools_list().await,
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
            "commands/list" => self.handle_commands_list().await,
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
                models.push(DesktopModel {
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
        let cfg = self
            .runtime
            .select_provider(params.provider, params.model, params.reasoning)
            .await
            .map_err(internal_error)?;
        if let Some(thread_id) = thread_id {
            self.desktop_thread_models.write().await.insert(
                thread_id.clone(),
                (cfg.default_provider.clone(), cfg.default_model.clone()),
            );
            if let Some(thread) = self.desktop_threads.write().await.get_mut(&thread_id) {
                thread.model_provider = cfg.default_provider.clone();
                thread.updated_at = OffsetDateTime::now_utc().unix_timestamp();
            }
        }
        if self.persist_user_config {
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
            reasoning: self.runtime.effective_reasoning().await,
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

    async fn handle_settings_get(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(SettingsGetResult {
            web_search: WebSearchSettings {
                mode: cfg.hosted_web_search.mode,
            },
            default_mode: cfg.policy_mode,
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

    async fn handle_codex_auth_login(&self) -> Result<serde_json::Value, JsonRpcError> {
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
        roder_codex_auth::logout().map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: false,
            account_id: None,
        })
        .unwrap())
    }

    async fn handle_supergrok_auth_login(&self) -> Result<serde_json::Value, JsonRpcError> {
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
        roder_supergrok_auth::logout().map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderAuthResult {
            signed_in: false,
            account_id: None,
        })
        .unwrap())
    }

    async fn handle_thread_list(
        &self,
        params: ThreadListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut sessions = self.runtime.list_sessions().await.map_err(internal_error)?;
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        if let Some(limit) = params.limit {
            sessions.truncate(limit);
        }
        let threads = sessions
            .into_iter()
            .map(|metadata| desktop_thread_from_metadata(metadata, None))
            .collect::<Vec<_>>();
        let mut threads = threads;
        for thread in self.desktop_threads.read().await.values() {
            if !threads.iter().any(|candidate| candidate.id == thread.id) {
                threads.push(thread.clone());
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
        let metadata = self
            .runtime
            .create_session_with(roder_core::CreateSessionRequest {
                title: None,
                workspace: params.cwd.clone(),
                provider: params.model_provider.clone(),
                model: params.model.clone(),
            })
            .await
            .map_err(internal_error)?;
        let cfg = self.runtime.status().await;
        let model = params.model.unwrap_or(cfg.default_model);
        let model_provider = params.model_provider.unwrap_or(cfg.default_provider);
        let cwd = params
            .cwd
            .or_else(|| metadata.workspace.clone())
            .unwrap_or_else(default_cwd_string);
        let thread = desktop_thread_from_metadata(metadata, None);
        self.desktop_threads
            .write()
            .await
            .insert(thread.id.clone(), thread.clone());
        self.desktop_thread_models
            .write()
            .await
            .insert(thread.id.clone(), (model_provider.clone(), model.clone()));
        let _ = self
            .desktop_notifications
            .send(notifications::thread_started_notification(thread.clone()));
        Ok(serde_json::to_value(ThreadStartResult {
            thread,
            model,
            model_provider,
            cwd,
        })
        .unwrap())
    }

    async fn handle_thread_read(
        &self,
        params: ThreadReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let thread = snapshot.and_then(|snapshot| {
            let turns = params.include_turns.then(|| {
                snapshot
                    .turns
                    .into_iter()
                    .map(desktop_turn_from_record)
                    .collect()
            });
            snapshot
                .metadata
                .map(|metadata| desktop_thread_from_metadata(metadata, turns))
        });
        let thread = if thread.is_some() {
            thread
        } else {
            self.runtime
                .list_sessions()
                .await
                .map_err(internal_error)?
                .into_iter()
                .find(|metadata| metadata.thread_id == params.thread_id)
                .map(|metadata| {
                    desktop_thread_from_metadata(metadata, params.include_turns.then(Vec::new))
                })
        };
        let thread = if thread.is_some() {
            thread
        } else {
            self.desktop_threads
                .read()
                .await
                .get(params.thread_id.as_str())
                .cloned()
                .map(|mut thread| {
                    if params.include_turns && thread.turns.is_none() {
                        thread.turns = Some(Vec::new());
                    }
                    thread
                })
        };
        Ok(serde_json::to_value(ThreadReadResult { thread }).unwrap())
    }

    async fn handle_thread_archive(
        &self,
        params: ThreadArchiveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let archived = self
            .runtime
            .archive_session(&params.thread_id)
            .await
            .map_err(internal_error)?;
        self.desktop_threads.write().await.remove(&params.thread_id);
        self.desktop_thread_models
            .write()
            .await
            .remove(&params.thread_id);
        self.desktop_active_turns
            .write()
            .await
            .remove(&params.thread_id);
        Ok(serde_json::to_value(ThreadArchiveResult {
            thread_id: params.thread_id,
            archived,
        })
        .unwrap())
    }

    async fn handle_session_get(&self) -> Result<serde_json::Value, JsonRpcError> {
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
        Ok(serde_json::to_value(SessionGetResult {
            mode: cfg.policy_mode,
            pending_plan_exit: pending,
        })
        .unwrap())
    }

    async fn handle_session_set_mode(
        &self,
        params: SessionSetModeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .set_policy_mode(params.mode, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(SessionSetModeResult {
            mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn handle_session_exit_plan(
        &self,
        params: SessionExitPlanParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_pending_plan_exit(&params.request_id, params.approved)
            .await
            .map_err(internal_error)?
            .is_some();
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(SessionExitPlanResult {
            resolved,
            mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn handle_session_resolve_approval(
        &self,
        params: SessionResolveApprovalParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_tool_approval(&params.approval_id, params.approved)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(SessionResolveApprovalResult { resolved }).unwrap())
    }

    async fn handle_session_resolve_user_input(
        &self,
        params: SessionResolveUserInputParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let resolved = self
            .runtime
            .resolve_user_input(&params.request_id, params.answers)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(SessionResolveUserInputResult { resolved }).unwrap())
    }

    async fn handle_desktop_turn_start(
        &self,
        params: TurnStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (provider_override, model_override) = self
            .desktop_thread_model(&params.thread_id)
            .await
            .map(|(provider, model)| (Some(provider), Some(model)))
            .unwrap_or((None, None));
        let mut workspace = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?
            .and_then(|snapshot| snapshot.metadata.and_then(|metadata| metadata.workspace));
        if workspace.is_none() {
            workspace = self
                .desktop_threads
                .read()
                .await
                .get(&params.thread_id)
                .map(|thread| thread.cwd.clone());
        }
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id.clone(),
                message: desktop_turn_message(&params.input, params.prompt),
                images: desktop_turn_images(&params.input),
                provider_override,
                model_override,
                workspace,
                instructions: default_instructions(),
            })
            .await
            .map_err(internal_error)?;
        self.desktop_active_turns
            .write()
            .await
            .insert(params.thread_id, turn_id.clone());
        Ok(serde_json::to_value(TurnStartResult { turn_id }).unwrap())
    }

    async fn handle_desktop_turn_interrupt(
        &self,
        params: TurnInterruptParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = if let Some(turn_id) = params.turn_id.clone() {
            turn_id
        } else {
            self.desktop_active_turns
                .read()
                .await
                .get(&params.thread_id)
                .cloned()
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
        self.desktop_active_turns
            .write()
            .await
            .remove(&params.thread_id);
        Ok(serde_json::to_value(TurnInterruptResult {
            turn_id: Some(turn_id),
        })
        .unwrap())
    }

    async fn handle_desktop_turn_steer(
        &self,
        params: TurnSteerParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = params.expected_turn_id;
        self.runtime
            .steer_turn(
                params.thread_id,
                turn_id.clone(),
                desktop_turn_message(&params.input, params.prompt),
                desktop_turn_images(&params.input),
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

    async fn desktop_thread_model(&self, thread_id: &str) -> Option<(String, String)> {
        if let Some(model) = self
            .desktop_thread_models
            .read()
            .await
            .get(thread_id)
            .cloned()
        {
            return Some(model);
        }
        self.runtime
            .list_sessions()
            .await
            .ok()?
            .into_iter()
            .find(|metadata| metadata.thread_id == thread_id)
            .and_then(|metadata| match (metadata.provider, metadata.model) {
                (Some(provider), Some(model)) => Some((provider, model)),
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
        let task = self
            .tasks
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
            .map_err(internal_error)?;
        Ok(serde_json::to_value(TasksSubmitResult { task }).unwrap())
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
        let workspace = params.workspace.clone();
        let expanded = self
            .expand_command(CommandsExpandParams {
                name: params.name,
                arguments: params.arguments,
                workspace: params.workspace,
            })
            .await?;
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id,
                message: expanded.message.clone(),
                images: Vec::new(),
                provider_override: None,
                model_override: expanded.model.clone(),
                workspace,
                instructions: default_instructions(),
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
        let cfg = roder_config::load_config()?.commands.unwrap_or_default();
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
        let user_dir = cfg.user_dir.as_deref().map(expand_tilde).or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".roder").join("commands"))
        });
        CommandsRegistry::load_with_options(
            user_dir.as_ref(),
            workspace_dir.as_ref(),
            std::iter::empty::<ExtensionCommandDirectory>(),
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
        if !matches!(params.tool_name.as_str(), "get_goal" | "create_goal") {
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
            .load_session(&params.thread_id)
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
            .load_session(&params.thread_id)
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
        if include_user && let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            options.user_roots.push(home.join(".roder"));
            options.user_roots.push(home.join(".agents"));
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
        let image = artifact_is_image(&artifact).then(|| roder_api::conversation::InputImage {
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
            .load_session(thread_id)
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
            .load_session(thread_id)
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
        self.desktop_notifications.subscribe()
    }

    pub(crate) fn publish_notification(&self, notification: JsonRpcNotification) {
        let _ = self.desktop_notifications.send(notification);
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

fn not_found(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
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
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("could not resolve HOME for workflow import state"))?;
    Ok(home.join(".roder").join("workflow-imports.json"))
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
        ProviderAuthType::ApiKey => (
            metadata.auth_configured.unwrap_or(true),
            metadata.auth_label.clone(),
        ),
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
