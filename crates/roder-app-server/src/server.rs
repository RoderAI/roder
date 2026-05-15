use std::sync::Arc;
use roder_core::Runtime;
use roder_protocol::{
    CreateSessionResult, JsonRpcError, JsonRpcRequest, JsonRpcResponse, StartTurnParams,
    StartTurnResult,
};
use roder_api::inference::{AgentInferenceRequest, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints};
use roder_api::events::EventEnvelope;
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
            "sessions/create" => self.handle_create_session().await,
            "turns/start" => {
                if let Some(params) = req.params {
                    match serde_json::from_value::<StartTurnParams>(params) {
                        Ok(p) => self.handle_start_turn(p).await,
                        Err(e) => Err(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {}", e),
                            data: None,
                        }),
                    }
                } else {
                    Err(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    })
                }
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

    async fn handle_create_session(&self) -> Result<serde_json::Value, JsonRpcError> {
        let thread_id = uuid::Uuid::new_v4().to_string();
        
        let result = CreateSessionResult {
            thread_id,
        };

        Ok(serde_json::to_value(result).unwrap())
    }

    async fn handle_start_turn(
        &self,
        params: StartTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: params.provider_override.unwrap_or_else(|| "fake-provider".to_string()),
                model: params.model_override.unwrap_or_else(|| "fake-model".to_string()),
            },
            instructions: InstructionBundle {
                system: None,
                developer: None,
            },
            conversation: vec![],
            tools: vec![],
            tool_choice: roder_api::tools::ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: false,
                level: None,
            },
            output: OutputConfig {
                max_tokens: None,
                temperature: None,
                top_p: None,
            },
            runtime: RuntimeHints {
                trace_id: None,
            },
        };

        match self.runtime.start_turn(params.thread_id, request).await {
            Ok(turn_id) => {
                let result = StartTurnResult { turn_id };
                Ok(serde_json::to_value(result).unwrap())
            }
            Err(e) => Err(JsonRpcError {
                code: -32000,
                message: e.to_string(),
                data: None,
            }),
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.runtime.bus.subscribe()
    }
}
