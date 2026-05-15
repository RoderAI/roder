use std::sync::atomic::{AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};

use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::inference::{InferenceProviderContext, InferenceProviderMetadata, ProviderAuthType};
use roder_api::notifications::{Notification, NotificationKind, NotificationSink};
use roder_commands::{
    CommandExpansionOptions, CommandExpansionRequest, CommandsRegistry, ExtensionCommandDirectory,
    expand_command,
};
use roder_core::{Runtime, StartTurnRequest, default_instructions};
use roder_protocol::*;
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};
use tokio::sync::broadcast;

pub struct AppServer {
    pub runtime: Arc<Runtime>,
    persist_user_config: bool,
    tasks: BackgroundRunner,
    events: broadcast::Sender<EventEnvelope>,
    event_seq: Arc<AtomicU64>,
}

impl AppServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let tasks = build_task_runner(&runtime);
        let (events, _) = broadcast::channel(1024);
        let server = Self {
            runtime,
            persist_user_config: false,
            tasks,
            events,
            event_seq: Arc::new(AtomicU64::new(0)),
        };
        server.spawn_event_bridges();
        server
    }

    pub fn with_user_config_persistence(mut self) -> Self {
        self.persist_user_config = true;
        self
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let result = match req.method.as_str() {
            "system/initialize" | "system/status" => self.handle_system_status().await,
            "extensions/list" => self.handle_extensions_list().await,
            "providers/list" => self.handle_providers_list().await,
            "providers/select" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_provider_select(p).await
                })
                .await
            }
            "auth/codex/login" => self.handle_codex_auth_login().await,
            "auth/codex/status" => self.handle_codex_auth_status().await,
            "auth/codex/logout" => self.handle_codex_auth_logout().await,
            "sessions/create" => {
                let params = req
                    .params
                    .map(serde_json::from_value::<CreateSessionParams>)
                    .transpose()
                    .map_err(invalid_params)
                    .map(|p| p.unwrap_or(CreateSessionParams { title: None }));
                match params {
                    Ok(params) => self.handle_create_session(params).await,
                    Err(err) => Err(err),
                }
            }
            "sessions/list" => self.handle_sessions_list().await,
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
            "sessions/load" | "sessions/resume" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_session_load(p).await },
                )
                .await
            }
            "turns/start" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_start_turn(p).await },
                )
                .await
            }
            "turns/interrupt" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_interrupt_turn(p).await
                })
                .await
            }
            "tools/list" => self.handle_tools_list().await,
            "agents/list" => self.handle_agents_list().await,
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
            "tasks/subscribe" => {
                Ok(serde_json::to_value(TasksSubscribeResult { subscribed: true }).unwrap())
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

    async fn handle_system_status(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(SystemStatusResult {
            provider: cfg.default_provider,
            model: cfg.default_model,
            extensions: self.runtime.registry().manifests.len(),
            providers: self.runtime.registry().inference_engines.len(),
        })
        .unwrap())
    }

    async fn handle_extensions_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ExtensionsListResult {
            extensions: self.runtime.registry().manifests.clone(),
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
            providers,
        })
        .unwrap())
    }

    async fn handle_provider_select(
        &self,
        params: ProviderSelectParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self
            .runtime
            .select_provider(params.provider, params.model)
            .await
            .map_err(internal_error)?;
        if self.persist_user_config {
            roder_config::save_default_provider_model(&cfg.default_provider, &cfg.default_model)
                .map_err(internal_error)?;
        }
        Ok(serde_json::to_value(ProviderSelectResult {
            provider: cfg.default_provider,
            model: cfg.default_model,
        })
        .unwrap())
    }

    async fn handle_codex_auth_login(&self) -> Result<serde_json::Value, JsonRpcError> {
        let tokens = roder_codex_auth::login().await.map_err(internal_error)?;
        Ok(serde_json::to_value(CodexAuthResult {
            signed_in: true,
            account_id: non_empty(tokens.account_id),
        })
        .unwrap())
    }

    async fn handle_codex_auth_status(&self) -> Result<serde_json::Value, JsonRpcError> {
        let signed_in = roder_codex_auth::status().await.map_err(internal_error)?;
        Ok(serde_json::to_value(CodexAuthResult {
            signed_in: signed_in.is_some(),
            account_id: signed_in.and_then(|tokens| non_empty(tokens.account_id)),
        })
        .unwrap())
    }

    async fn handle_codex_auth_logout(&self) -> Result<serde_json::Value, JsonRpcError> {
        roder_codex_auth::logout().map_err(internal_error)?;
        Ok(serde_json::to_value(CodexAuthResult {
            signed_in: false,
            account_id: None,
        })
        .unwrap())
    }

    async fn handle_create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let metadata = self
            .runtime
            .create_session(params.title)
            .await
            .map_err(internal_error)?;
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(CreateSessionResult {
            thread_id: metadata.thread_id,
            provider: cfg.default_provider,
            model: cfg.default_model,
        })
        .unwrap())
    }

    async fn handle_sessions_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let sessions = self.runtime.list_sessions().await.map_err(internal_error)?;
        Ok(serde_json::to_value(SessionsListResult { sessions }).unwrap())
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
        let pending_tool_approval = self.runtime.pending_tool_approval().await.map(|pending| {
            PendingToolApprovalDescriptor {
                thread_id: pending.thread_id,
                turn_id: pending.turn_id,
                approval_id: pending.approval_id,
                tool_id: pending.tool_id,
                tool_name: pending.tool_name,
                reason: pending.reason,
                requested_at: pending.requested_at,
            }
        });
        Ok(serde_json::to_value(SessionGetResult {
            mode: cfg.policy_mode,
            pending_plan_exit: pending,
            pending_tool_approval,
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
            .resolve_pending_tool_approval(&params.approval_id, params.approved)
            .await
            .map_err(internal_error)?
            .is_some();
        Ok(serde_json::to_value(SessionResolveApprovalResult { resolved }).unwrap())
    }

    async fn handle_session_load(
        &self,
        params: SessionLoadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(SessionLoadResult { snapshot }).unwrap())
    }

    async fn handle_start_turn(
        &self,
        params: StartTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id,
                message: params.message,
                provider_override: params.provider_override,
                model_override: params.model_override,
                instructions: default_instructions(),
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(StartTurnResult { turn_id }).unwrap())
    }

    async fn handle_interrupt_turn(
        &self,
        params: InterruptTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.runtime
            .interrupt_turn(params.thread_id, params.turn_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::json!({}))
    }

    async fn handle_tools_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ToolsListResult {
            tools: self.runtime.tool_specs(),
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

    async fn handle_commands_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let registry = self.command_registry().await.map_err(internal_error)?;
        Ok(serde_json::to_value(CommandsListResult {
            commands: registry
                .iter()
                .map(|(_, spec)| CommandDescriptor {
                    name: spec.name.clone(),
                    description: spec.description.clone(),
                    argument_hint: spec.argument_hint.clone(),
                    source: spec.display_source(),
                    model: spec.model.clone(),
                    agent: spec.agent.clone(),
                    has_shell_includes: !spec.include.shell.is_empty(),
                    has_url_includes: !spec.include.urls.is_empty(),
                })
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
        let expanded = self
            .expand_command(CommandsExpandParams {
                name: params.name,
                arguments: params.arguments,
            })
            .await?;
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id,
                message: expanded.message.clone(),
                provider_override: None,
                model_override: expanded.model.clone(),
                instructions: default_instructions(),
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(CommandsRunResult { turn_id, expanded }).unwrap())
    }

    async fn handle_tasks_submit(
        &self,
        params: TasksSubmitParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let task = self
            .tasks
            .submit(
                params.executor_id,
                params.input,
                TaskSubmitOptions {
                    thread_id: params.thread_id,
                    turn_id: params.turn_id,
                    workspace_root: cfg.workspace,
                    deadline: None,
                    metadata: serde_json::json!({}),
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
        let task = self.tasks.get(&params.task_id).await;
        let (logs, dropped_bytes) = self
            .tasks
            .logs(&params.task_id)
            .await
            .unwrap_or_else(|| (Vec::new(), 0));
        Ok(serde_json::to_value(TasksGetResult {
            task,
            logs: logs
                .into_iter()
                .map(|entry| TaskLogEntryDescriptor {
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

    async fn expand_command(
        &self,
        params: CommandsExpandParams,
    ) -> Result<CommandsExpandResult, JsonRpcError> {
        let registry = self.command_registry().await.map_err(internal_error)?;
        let spec = registry.get(&params.name).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: format!("Unknown command `{}`", params.name),
            data: None,
        })?;
        let cfg = self.runtime.status().await;
        let workspace_root = cfg
            .workspace
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let expanded = expand_command(CommandExpansionRequest {
            spec,
            arguments: params.arguments.as_deref().unwrap_or_default(),
            workspace_root: &workspace_root,
            options: CommandExpansionOptions {
                policy_mode: cfg.policy_mode,
                ..CommandExpansionOptions::default()
            },
            shell_runner: None,
            url_fetcher: None,
        })
        .map_err(internal_error)?;
        Ok(CommandsExpandResult {
            name: expanded.command_name,
            message: expanded.message,
            context_blocks: expanded.context_blocks,
            allowed_tools: expanded.allowed_tools,
            model: expanded.model,
            agent: expanded.agent,
        })
    }

    async fn command_registry(&self) -> anyhow::Result<CommandsRegistry> {
        let cfg = self.runtime.status().await;
        let workspace_dir = cfg
            .workspace
            .as_ref()
            .map(|workspace| PathBuf::from(workspace).join(".roder").join("commands"));
        let user_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".roder").join("commands"));
        CommandsRegistry::load(
            user_dir.as_ref(),
            workspace_dir.as_ref(),
            std::iter::empty::<ExtensionCommandDirectory>(),
        )
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.events.subscribe()
    }

    fn spawn_event_bridges(&self) {
        let mut runtime_rx = self.runtime.subscribe_events();
        let runtime_events = self.events.clone();
        let notification_sinks = self.runtime.registry().notification_sinks.clone();
        tokio::spawn(async move {
            while let Ok(envelope) = runtime_rx.recv().await {
                deliver_notification_for_event(&notification_sinks, &envelope.event).await;
                let _ = runtime_events.send(envelope);
            }
        });

        let mut task_rx = self.tasks.subscribe();
        let task_events = self.events.clone();
        let notification_sinks = self.runtime.registry().notification_sinks.clone();
        let seq = Arc::clone(&self.event_seq);
        tokio::spawn(async move {
            while let Ok(event) = task_rx.recv().await {
                deliver_notification_for_event(&notification_sinks, &event).await;
                let envelope = EventEnvelope {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    seq: seq.fetch_add(1, Ordering::SeqCst) + 1,
                    timestamp: time::OffsetDateTime::now_utc(),
                    source: event.source(),
                    kind: event.kind().to_string(),
                    thread_id: event.thread_id().cloned(),
                    turn_id: event.turn_id().cloned(),
                    event,
                };
                let _ = task_events.send(envelope);
            }
        });
    }
}

fn build_task_runner(runtime: &Runtime) -> BackgroundRunner {
    let mut registry = TaskExecutorRegistry::default();
    for executor in &runtime.registry().task_executors {
        let _ = registry.register(executor.clone());
    }
    BackgroundRunner::new(registry, BackgroundRunnerConfig::default())
}

async fn deliver_notification_for_event(sinks: &[Arc<dyn NotificationSink>], event: &RoderEvent) {
    let notification = match event {
        RoderEvent::ApprovalRequested(ev) => Some(Notification {
            id: format!("approval-{}", ev.approval_id),
            kind: NotificationKind::NeedsInput,
            title: "Approval needed".to_string(),
            body: Some(ev.tool_name.clone()),
            task_id: None,
            thread_id: Some(ev.thread_id.clone()),
            turn_id: Some(ev.turn_id.clone()),
            timestamp: ev.timestamp,
            metadata: serde_json::json!({ "approval_id": ev.approval_id }),
        }),
        RoderEvent::TurnCompleted(ev) => Some(Notification {
            id: format!("turn-{}", ev.turn_id),
            kind: NotificationKind::TurnIdle,
            title: "Turn completed".to_string(),
            body: None,
            task_id: None,
            thread_id: Some(ev.thread_id.clone()),
            turn_id: Some(ev.turn_id.clone()),
            timestamp: ev.timestamp,
            metadata: serde_json::json!({}),
        }),
        RoderEvent::TaskCompleted(ev) => Some(Notification {
            id: format!("task-{}", ev.task_id),
            kind: NotificationKind::TaskCompleted,
            title: "Task completed".to_string(),
            body: Some(ev.task_id.clone()),
            task_id: Some(ev.task_id.clone()),
            thread_id: ev.thread_id.clone(),
            turn_id: ev.turn_id.clone(),
            timestamp: ev.timestamp,
            metadata: ev.payload.clone(),
        }),
        RoderEvent::TaskFailed(ev) => Some(Notification {
            id: format!("task-{}", ev.task_id),
            kind: NotificationKind::TaskFailed,
            title: "Task failed".to_string(),
            body: Some(ev.error.clone()),
            task_id: Some(ev.task_id.clone()),
            thread_id: ev.thread_id.clone(),
            turn_id: ev.turn_id.clone(),
            timestamp: ev.timestamp,
            metadata: serde_json::json!({ "error": ev.error }),
        }),
        _ => None,
    };
    let Some(notification) = notification else {
        return;
    };
    for sink in sinks {
        let _ = sink.deliver(notification.clone()).await;
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
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}

async fn provider_auth_status(
    provider_id: &str,
    metadata: &InferenceProviderMetadata,
) -> (bool, Option<String>) {
    match metadata.auth_type {
        ProviderAuthType::None => (true, None),
        ProviderAuthType::ApiKey => (true, metadata.auth_label.clone()),
        ProviderAuthType::OAuth if provider_id == roder_api::catalog::PROVIDER_CODEX => {
            match roder_codex_auth::status().await {
                Ok(Some(tokens)) if !tokens.account_id.is_empty() => {
                    (true, Some(tokens.account_id))
                }
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
