use std::path::PathBuf;

use futures::StreamExt;
use roder_api::catalog::PROVIDER_CURSOR;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, ReasoningDelta,
    TokenUsage, ToolCallCompleted,
};
use roder_api::transcript::TranscriptItem;
use serde_json::json;

use crate::agentservice::{
    AgentServiceConfig, AgentServiceEvent, AgentServiceRequest, stream_agent_service,
};
use crate::auth::CursorAuthConfig;
use crate::context::{
    CursorContextOptions, discovery_context_frames_from_env, encode_request_context_frame,
};
use crate::models::fallback_models;

#[derive(Debug, Clone, Default)]
pub struct CursorConfig {
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub agent_service_url: Option<String>,
    pub backend_base_url: Option<String>,
    pub workspace: Option<PathBuf>,
}

pub struct CursorInferenceEngine {
    config: CursorConfig,
}

impl CursorInferenceEngine {
    pub fn new(config: CursorConfig) -> Self {
        Self { config }
    }

    fn auth_config(&self) -> CursorAuthConfig {
        CursorAuthConfig {
            api_key: self.config.api_key.clone(),
            access_token: self.config.access_token.clone(),
            backend_base_url: self.config.backend_base_url.clone(),
        }
    }

    fn agent_service_config(&self) -> AgentServiceConfig {
        AgentServiceConfig {
            endpoint: self
                .config
                .agent_service_url
                .clone()
                .or_else(|| std::env::var("RODER_CURSOR_AGENT_SERVICE_URL").ok())
                .or_else(|| std::env::var("CURSOR_AGENT_SERVICE_URL").ok())
                .unwrap_or_else(|| crate::agentservice::DEFAULT_AGENT_SERVICE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            ..AgentServiceConfig::default()
        }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for CursorInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_CURSOR.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: true,
            structured_output: false,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Cursor".to_string(),
            description: Some("Cursor Composer via direct AgentService API".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("CURSOR_API_KEY or RODER_CURSOR_API_KEY".to_string()),
            auth_configured: Some(self.auth_config().has_auth()),
            recommended: true,
            sort_order: 18,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(fallback_models())
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        validate_request(&request)?;
        let auth = self.auth_config().resolve_access_token().await?;
        let workspace = self
            .config
            .workspace
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let context_frames = discovery_context_frames_from_env()?.unwrap_or_else(|| {
            vec![encode_request_context_frame(
                &CursorContextOptions::from_workspace(workspace),
            )]
        });
        let prompt = prompt_from_request(&request);
        let estimated_prompt_tokens = estimate_prompt_tokens(&prompt);
        let service_stream = stream_agent_service(
            self.agent_service_config(),
            AgentServiceRequest {
                access_token: auth.token,
                prompt,
                model: request.model.model.clone(),
                context_frames,
            },
        )
        .await?;
        let request_id = service_stream.request_id;
        let conversation_id = service_stream.conversation_id;
        let auth_source = auth.source.as_str().to_string();
        let thread_id = ctx.thread_id.to_string();
        let turn_id = ctx.turn_id.to_string();
        let model = request.model.model.clone();
        let mut service_events = service_stream.events;

        Ok(Box::pin(async_stream::try_stream! {
            let mut usage_fields = serde_json::Map::new();
            let mut estimated_visible_output_tokens = 0_u32;
            let mut estimated_thinking_tokens = 0_u32;

            while let Some(event) = service_events.next().await {
                match event? {
                    AgentServiceEvent::Text(text) => {
                        estimated_visible_output_tokens = estimated_visible_output_tokens
                            .saturating_add(estimate_text_tokens(&text));
                        yield InferenceEvent::MessageDelta(MessageDelta {
                            text,
                            phase: None,
                        });
                    }
                    AgentServiceEvent::Thinking(text) => {
                        estimated_thinking_tokens = estimated_thinking_tokens
                            .saturating_add(estimate_text_tokens(&text));
                        yield InferenceEvent::ReasoningDelta(ReasoningDelta { text });
                    }
                    AgentServiceEvent::ToolCalls(calls) => {
                        for call in calls {
                            estimated_visible_output_tokens = estimated_visible_output_tokens
                                .saturating_add(estimate_text_tokens(&call.arguments));
                            yield InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                                id: call.id,
                                name: call.name,
                                arguments: call.arguments,
                            });
                        }
                    }
                    AgentServiceEvent::UsageFields(fields) => {
                        for (field, value) in fields {
                            usage_fields.insert(format!("field_{field}"), json!(value));
                        }
                    }
                    AgentServiceEvent::Completed(stop_reason) => {
                        let completion_tokens = estimated_visible_output_tokens
                            .saturating_add(estimated_thinking_tokens);
                        let total_tokens = estimated_prompt_tokens.saturating_add(completion_tokens);
                        yield InferenceEvent::Usage(TokenUsage::new(
                            estimated_prompt_tokens,
                            completion_tokens,
                            total_tokens,
                        ));
                        yield InferenceEvent::ProviderMetadata(json!({
                            "provider": PROVIDER_CURSOR,
                            "transport": "cursor-agentservice-http2-connect-proto",
                            "authSource": auth_source,
                            "requestId": request_id,
                            "conversationId": conversation_id,
                            "threadId": thread_id,
                            "turnId": turn_id,
                            "model": model,
                            "usage": {
                                "input_tokens": estimated_prompt_tokens,
                                "output_tokens": completion_tokens,
                                "total_tokens": total_tokens,
                                "output_tokens_details": {
                                    "reasoning_tokens": estimated_thinking_tokens,
                                    "visible_output_tokens": estimated_visible_output_tokens
                                }
                            },
                            "usageFields": usage_fields,
                            "usageEstimated": true,
                            "usageSource": "chars_per_4",
                        }));
                        yield InferenceEvent::Completed(CompletionMetadata {
                            stop_reason: Some(stop_reason),
                            provider_response_id: None,
                        });
                        break;
                    }
                }
            }
        }))
    }
}

pub fn prompt_from_request(request: &AgentInferenceRequest) -> String {
    let mut sections = Vec::new();
    if let Some(system) = &request.instructions.system {
        sections.push(format!("System:\n{system}"));
    }
    if let Some(developer) = &request.instructions.developer {
        sections.push(format!("Developer:\n{developer}"));
    }
    for item in &request.transcript {
        match item {
            TranscriptItem::UserMessage(message) => {
                sections.push(format!("User:\n{}", message.text));
            }
            TranscriptItem::AssistantMessage(message) if !message.text.is_empty() => {
                sections.push(format!("Assistant:\n{}", message.text));
            }
            TranscriptItem::ContextCompaction(compaction) => {
                sections.push(format!("Context summary:\n{}", compaction.summary));
            }
            TranscriptItem::ReasoningSummary(summary) => {
                sections.push(format!("Reasoning summary:\n{}", summary.text));
            }
            TranscriptItem::ToolResult(result) => {
                sections.push(format!("Tool result {}:\n{}", result.id, result.result));
            }
            TranscriptItem::ToolCall(call) => {
                sections.push(format!(
                    "Assistant tool call {} {}:\n{}",
                    call.id, call.name, call.arguments
                ));
            }
            TranscriptItem::FileChange(_)
            | TranscriptItem::Error(_)
            | TranscriptItem::ProviderMetadata(_) => {}
            _ => {}
        }
    }
    sections.join("\n\n")
}

fn validate_request(request: &AgentInferenceRequest) -> anyhow::Result<()> {
    if request.output.response_format.is_some() {
        anyhow::bail!("Cursor provider does not support structured response_format yet");
    }
    Ok(())
}

fn estimate_prompt_tokens(prompt: &str) -> u32 {
    estimate_text_tokens(prompt).max(1)
}

fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::ToolChoice;
    use roder_api::transcript::{ToolCallRecord, ToolResultRecord, UserMessage};

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_CURSOR.to_string(),
                model: "composer-2.5".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("be useful".to_string()),
                developer: None,
            },
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn metadata_reports_api_key_auth_state() {
        let engine = CursorInferenceEngine::new(CursorConfig {
            api_key: Some("crsr_test".to_string()),
            ..CursorConfig::default()
        });
        let metadata = engine.metadata();
        assert_eq!(metadata.name, "Cursor");
        assert_eq!(metadata.auth_configured, Some(true));
    }

    #[test]
    fn prompt_mapping_preserves_system_and_user_text() {
        let prompt = prompt_from_request(&request());
        assert!(prompt.contains("System:\nbe useful"));
        assert!(prompt.contains("User:\nhello"));
    }

    #[test]
    fn prompt_mapping_replays_tool_call_and_result_for_next_cursor_round() {
        let mut request = request();
        request
            .transcript
            .push(TranscriptItem::ToolCall(ToolCallRecord {
                id: "tool_read_123".to_string(),
                name: "read_file".to_string(),
                arguments: r#"{"path":"AGENTS.md"}"#.to_string(),
            }));
        request
            .transcript
            .push(TranscriptItem::ToolResult(ToolResultRecord {
                id: "tool_read_123".to_string(),
                name: Some("read_file".to_string()),
                result: "first line".to_string(),
                display_payload: None,
                is_error: false,
            }));

        let prompt = prompt_from_request(&request);

        assert!(prompt.contains("Assistant tool call tool_read_123 read_file:"));
        assert!(prompt.contains(r#"{"path":"AGENTS.md"}"#));
        assert!(prompt.contains("Tool result tool_read_123:\nfirst line"));
    }

    #[test]
    fn validation_allows_roder_tool_requests() {
        let mut request = request();
        request.tools.push(roder_api::tools::ToolSpec {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        });
        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn token_estimator_produces_nonzero_prompt_and_ceil_output_counts() {
        assert_eq!(estimate_prompt_tokens("abc"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
    }
}
