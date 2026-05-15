use std::sync::Arc;

use roder_api::events::EventEnvelope;
use roder_api::inference::{InferenceProviderContext, InstructionBundle};
use roder_core::{Runtime, StartTurnRequest};
use roder_protocol::*;
use tokio::sync::broadcast;

pub struct AppServer {
    pub runtime: Arc<Runtime>,
}

impl AppServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
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
            let models = engine
                .list_models(InferenceProviderContext { provider_id: &id })
                .await
                .unwrap_or_default();
            providers.push(ProviderDescriptor {
                id,
                capabilities: engine.capabilities(),
                models,
            });
        }
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
                instructions: InstructionBundle::default(),
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
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}
