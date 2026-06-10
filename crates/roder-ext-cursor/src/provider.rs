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
use crate::proto::CursorHistoryMessage;

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
            tool_search: false,
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

        // When the runtime supplies a tool executor, drive Cursor's bidirectional
        // agent runtime: keep the Run stream open and service exec read/write/shell
        // requests in-stream so the model completes multi-step edits in one turn.
        if let Some(executor) = ctx.tool_executor.clone() {
            let (prompt, history) = cursor_request_parts(&request);
            let conversation_id = uuid::Uuid::new_v4().to_string();
            let message_id = uuid::Uuid::new_v4().to_string();
            let run_request = crate::proto::encode_agent_client_message_with_history(
                &prompt,
                &request.model.model,
                &conversation_id,
                &message_id,
                &history,
            );
            let context_frames = discovery_context_frames_from_env()?.unwrap_or_else(|| {
                vec![encode_request_context_frame(
                    &CursorContextOptions::from_workspace(workspace.clone()),
                )]
            });
            return crate::bidi::run_bidi_turn(
                self.agent_service_config(),
                crate::bidi::BidiRequest {
                    access_token: auth.token,
                    run_request,
                    context_frames,
                    workspace,
                    tool_executor: Some(executor),
                },
            )
            .await;
        }

        let workspace_for_ctx = workspace.clone();
        let context_frames = discovery_context_frames_from_env()?.unwrap_or_else(|| {
            vec![encode_request_context_frame(
                &CursorContextOptions::from_workspace(workspace_for_ctx),
            )]
        });
        let (prompt, history) = cursor_request_parts(&request);
        let estimated_prompt_tokens = estimate_prompt_tokens(&prompt);
        let service_stream = stream_agent_service(
            self.agent_service_config(),
            AgentServiceRequest {
                access_token: auth.token,
                prompt,
                model: request.model.model.clone(),
                context_frames,
                history,
                workspace,
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

/// Split a Roder inference request into the Cursor `user_message` text (system +
/// developer + the latest user turn) and the native `ConversationHistory` (all
/// prior turns, including the assistant tool calls and tool results from earlier
/// Roder rounds). Replaying the tool calls/results natively lets Cursor's agent
/// continue the loop instead of restarting and re-issuing the same tool call.
pub fn cursor_request_parts(
    request: &AgentInferenceRequest,
) -> (String, Vec<CursorHistoryMessage>) {
    let last_user_idx = request
        .transcript
        .iter()
        .rposition(|item| matches!(item, TranscriptItem::UserMessage(_)));

    let mut history = Vec::new();
    let mut current_user_text = String::new();
    for (idx, item) in request.transcript.iter().enumerate() {
        match item {
            TranscriptItem::UserMessage(message) => {
                if Some(idx) == last_user_idx {
                    current_user_text = message.text.clone();
                } else {
                    history.push(CursorHistoryMessage::User(message.text.clone()));
                }
            }
            TranscriptItem::AssistantMessage(message) if !message.text.is_empty() => {
                history.push(CursorHistoryMessage::AssistantText(message.text.clone()));
            }
            TranscriptItem::ToolCall(call) => {
                history.push(CursorHistoryMessage::AssistantToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    args_json: call.arguments.clone(),
                });
            }
            TranscriptItem::ToolResult(result) => {
                history.push(CursorHistoryMessage::ToolResult {
                    id: result.id.clone(),
                    name: result.name.clone().unwrap_or_default(),
                    content: result.result.clone(),
                    is_error: result.is_error,
                });
            }
            TranscriptItem::ContextCompaction(compaction) => {
                history.push(CursorHistoryMessage::User(format!(
                    "Context summary:\n{}",
                    compaction.summary
                )));
            }
            _ => {}
        }
    }

    let mut sections = Vec::new();
    if let Some(system) = &request.instructions.system {
        sections.push(format!("System:\n{system}"));
    }
    if let Some(developer) = &request.instructions.developer {
        sections.push(format!("Developer:\n{developer}"));
    }
    if !current_user_text.is_empty() {
        sections.push(current_user_text);
    }
    // Cursor's agent is built for same-stream tool loops and otherwise restarts
    // its read-before-act pattern on each fresh Roder round. When prior tool
    // results are already in the conversation history, steer the model to act on
    // them instead of re-issuing the same read-only calls.
    let has_tool_results = history
        .iter()
        .any(|item| matches!(item, CursorHistoryMessage::ToolResult { .. }));
    if has_tool_results {
        sections.push(
            "Continuation: the conversation history above already contains the tool calls you made and their results. Do not repeat read-only tool calls (read_file, grep, glob, ls) for information you already have. Use the results above and proceed directly to the remaining action (e.g. the edit or shell command) that completes the task. If the task is already complete, give the final answer."
                .to_string(),
        );
    }
    (sections.join("\n\n"), history)
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
    fn request_parts_keep_instructions_and_latest_user_message_in_prompt() {
        let (prompt, history) = cursor_request_parts(&request());
        assert!(prompt.contains("System:\nbe useful"));
        assert!(prompt.contains("hello"));
        // A single fresh user turn has no prior history.
        assert!(history.is_empty());
    }

    #[test]
    fn request_parts_replay_tool_call_and_result_as_native_history() {
        let mut request = request();
        request
            .transcript
            .push(TranscriptItem::ToolCall(ToolCallRecord {
                id: "toolu_read_123".to_string(),
                name: "read_file".to_string(),
                arguments: r#"{"path":"AGENTS.md"}"#.to_string(),
            }));
        request
            .transcript
            .push(TranscriptItem::ToolResult(ToolResultRecord {
                id: "toolu_read_123".to_string(),
                name: Some("read_file".to_string()),
                result: "first line".to_string(),
                display_payload: None,
                is_error: false,
            }));

        let (prompt, history) = cursor_request_parts(&request);

        // The original user request stays the current prompt...
        assert!(prompt.contains("hello"));
        // ...and the tool call + result are replayed as native history.
        assert_eq!(history.len(), 2);
        assert!(matches!(
            &history[0],
            CursorHistoryMessage::AssistantToolCall { id, name, args_json }
                if id == "toolu_read_123" && name == "read_file" && args_json.contains("AGENTS.md")
        ));
        assert!(matches!(
            &history[1],
            CursorHistoryMessage::ToolResult { id, content, is_error, .. }
                if id == "toolu_read_123" && content == "first line" && !*is_error
        ));
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
