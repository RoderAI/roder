use std::sync::Arc;

use roder_api::events::EventEnvelope;
use roder_api::inference::{
    HostedWebSearchMode, InferenceProviderContext, InferenceProviderMetadata, ProviderAuthType,
};
use roder_core::{Runtime, StartTurnRequest, default_instructions};
use roder_protocol::*;
use tokio::sync::broadcast;

pub struct AppServer {
    pub runtime: Arc<Runtime>,
    persist_user_config: bool,
}

impl AppServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            persist_user_config: false,
        }
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
            "sessions/create" => {
                let params = req
                    .params
                    .map(serde_json::from_value::<CreateSessionParams>)
                    .transpose()
                    .map_err(invalid_params)
                    .map(|p| {
                        p.unwrap_or(CreateSessionParams {
                            title: None,
                            workspace: None,
                            provider: None,
                            model: None,
                        })
                    });
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
            "session/resolve_user_input" => {
                self.decode_and(req.params, |p| async move {
                    self.handle_session_resolve_user_input(p).await
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
            "turns/steer" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_steer_turn(p).await },
                )
                .await
            }
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => {
                self.decode_and(
                    req.params,
                    |p| async move { self.handle_tool_call(p).await },
                )
                .await
            }
            "agents/list" => self.handle_agents_list().await,
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
            reasoning: self.runtime.effective_reasoning().await,
            web_search: WebSearchSettings {
                mode: cfg.hosted_web_search.mode,
            },
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
            active_reasoning: self.runtime.effective_reasoning().await,
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
            .select_provider(params.provider, params.model, params.reasoning)
            .await
            .map_err(internal_error)?;
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
            .create_session_with(roder_core::CreateSessionRequest {
                title: params.title,
                workspace: params.workspace,
                provider: params.provider,
                model: params.model,
            })
            .await
            .map_err(internal_error)?;
        let cfg = self.runtime.status().await;
        Ok(serde_json::to_value(CreateSessionResult {
            thread_id: metadata.thread_id,
            provider: cfg.default_provider,
            model: cfg.default_model,
            reasoning: self.runtime.effective_reasoning().await,
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
                images: params.images,
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

    async fn handle_steer_turn(
        &self,
        params: SteerTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let turn_id = params.turn_id;
        self.runtime
            .steer_turn(
                params.thread_id,
                turn_id.clone(),
                params.message,
                params.images,
            )
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(SteerTurnResult { turn_id }).unwrap())
    }

    async fn handle_tools_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ToolsListResult {
            tools: self.runtime.tool_specs().await,
        })
        .unwrap())
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

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.runtime.subscribe_events()
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

fn web_search_mode_config_value(mode: HostedWebSearchMode) -> &'static str {
    match mode {
        HostedWebSearchMode::Disabled => "disabled",
        HostedWebSearchMode::Cached => "codex",
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
