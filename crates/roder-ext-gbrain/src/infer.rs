//! [`Reasoner`] backed by a roder [`InferenceEngine`].
//!
//! The decision loop needs a one-shot `complete(system, user) -> text` seam. Rather
//! than re-implement provider HTTP clients (Anthropic, OpenAI Responses, …), this
//! drives roder's own inference primitive — the same `InferenceEngine` the rest of
//! roder uses — so the gbrain extension inherits every provider roder ships
//! (`roder-ext-anthropic`, `roder-ext-openai-responses`, …) and their reasoning /
//! retry / reliability handling for free. The engines construct standalone
//! (`Engine::new(api_key)`), so this works in the CLI / eval without an extension
//! registry; in-process callers can pass any registry-provided engine instead.

use std::sync::Arc;

use futures::{StreamExt, future::join_all};
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, RuntimeProfile,
    TokenUsage,
};
use roder_api::memory::MemoryScope;
use roder_api::tools::{ToolChoice, ToolRegistry};
use roder_api::transcript::{
    AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    tool_display_payload,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::agent::prompts::agentic_retrieval_system_prompt;
use crate::agent::retriever::{ModelSelectedToolCall, ToolObservation};
use crate::reason::{Completion, Reasoner};
use crate::store::MemorySnapshotReport;

mod agentic_helpers;

use agentic_helpers::{
    agentic_user_prompt, apply_default_scope, capture_trace_fields, cited_evidence_ids,
    execute_agentic_tool, parse_tool_arguments,
};

/// Wraps any roder [`InferenceEngine`] as a one-shot [`Reasoner`].
pub struct EngineReasoner {
    engine: Arc<dyn InferenceEngine>,
    provider: String,
    model: String,
    /// Reasoning effort level (e.g. "medium"); `None` disables reasoning.
    reasoning_level: Option<String>,
    max_tokens: u32,
}

impl EngineReasoner {
    pub fn new(
        engine: Arc<dyn InferenceEngine>,
        provider: impl Into<String>,
        model: impl Into<String>,
        reasoning_level: Option<String>,
    ) -> Self {
        Self {
            engine,
            provider: provider.into(),
            model: model.into(),
            reasoning_level,
            max_tokens: 16000,
        }
    }
}

#[async_trait::async_trait]
impl Reasoner for EngineReasoner {
    fn label(&self) -> String {
        match &self.reasoning_level {
            Some(l) => format!("{}/{} (reasoning={l})", self.provider, self.model),
            None => format!("{}/{}", self.provider, self.model),
        }
    }

    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<Completion> {
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: self.provider.clone(),
                model: self.model.clone(),
            },
            instructions: InstructionBundle {
                system: Some(system.to_string()),
                developer: None,
            },
            transcript: vec![TranscriptItem::UserMessage(UserMessage {
                text: user.to_string(),
                images: Vec::new(),
            })],
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
            reasoning: ReasoningConfig {
                enabled: self.reasoning_level.is_some(),
                level: self.reasoning_level.clone(),
            },
            output: OutputConfig {
                max_tokens: Some(self.max_tokens),
                ..Default::default()
            },
            runtime: RuntimeHints {
                profile: RuntimeProfile::Eval,
                ..Default::default()
            },
            metadata: serde_json::Value::Null,
        };
        let ctx = InferenceTurnContext {
            thread_id: "gbrain",
            turn_id: "answer",
            tool_executor: None,
        };
        let mut stream = self.engine.stream_turn(ctx, request).await?;
        let mut text = String::new();
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        while let Some(event) = stream.next().await {
            match event? {
                InferenceEvent::MessageDelta(d) => text.push_str(&d.text),
                InferenceEvent::Usage(u) => {
                    input_tokens = u.prompt_tokens;
                    output_tokens = u.completion_tokens;
                }
                InferenceEvent::Failed(f) => anyhow::bail!("inference failed: {}", f.message),
                _ => {}
            }
        }
        Ok(Completion {
            text,
            input_tokens,
            output_tokens,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AgenticToolRunnerConfig {
    pub max_tool_calls: Option<usize>,
    pub max_tokens: u32,
    pub parallel_tool_calls: bool,
}

impl Default for AgenticToolRunnerConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: None,
            max_tokens: 16000,
            parallel_tool_calls: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgenticToolAnswer {
    pub answer: String,
    pub provenance: Vec<String>,
    pub trace: AgenticToolTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgenticToolTrace {
    pub mode: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_snapshot_high_watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_dream_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_ontology_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_snapshot_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_snapshot: Option<MemorySnapshotReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_trace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_feedback_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    pub request_mutations_allowed: bool,
    pub parallel_tool_calls: bool,
    pub provider_turns: Vec<AgenticProviderTurnTrace>,
    pub tool_calls: Vec<ModelSelectedToolCall>,
    pub tool_observations: Vec<ToolObservation>,
    pub retrieval_notes: Vec<Value>,
    pub responded_via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_confidence: Option<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub claims: Vec<Value>,
    #[serde(default)]
    pub rejected_claims: Vec<Value>,
    pub unsupported_claim_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_span_coverage: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_precision: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgenticProviderTurnTrace {
    pub turn_index: usize,
    pub transcript_items: usize,
    pub message: String,
    pub tool_calls: Vec<ModelSelectedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Provider-native tool runner for `roder-gbrain answer --agentic-tools`.
pub struct EngineAgenticToolRunner {
    engine: Arc<dyn InferenceEngine>,
    provider: String,
    model: String,
    reasoning_level: Option<String>,
    config: AgenticToolRunnerConfig,
}

impl EngineAgenticToolRunner {
    pub fn new(
        engine: Arc<dyn InferenceEngine>,
        provider: impl Into<String>,
        model: impl Into<String>,
        reasoning_level: Option<String>,
    ) -> Self {
        Self {
            engine,
            provider: provider.into(),
            model: model.into(),
            reasoning_level,
            config: AgenticToolRunnerConfig::default(),
        }
    }

    pub fn with_config(mut self, config: AgenticToolRunnerConfig) -> Self {
        self.config = config;
        self
    }

    pub async fn answer_with_tools(
        &self,
        registry: ToolRegistry,
        question: &str,
        scope: Option<MemoryScope>,
        as_of: Option<OffsetDateTime>,
    ) -> anyhow::Result<AgenticToolAnswer> {
        let mut transcript = vec![TranscriptItem::UserMessage(UserMessage::text(
            agentic_user_prompt(question, scope.as_ref(), as_of),
        ))];
        let mut trace = AgenticToolTrace {
            mode: "provider_tool_agentic".to_string(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            raw_snapshot_high_watermark: None,
            selected_dream_run_id: None,
            selected_ontology_version: None,
            derived_snapshot_version: None,
            memory_snapshot: None,
            full_trace_path: None,
            query_feedback_id: None,
            scope_id: scope.as_ref().map(MemoryScope::stable_id),
            as_of: as_of.map(|value| value.date().to_string()),
            request_mutations_allowed: false,
            parallel_tool_calls: self.config.parallel_tool_calls,
            provider_turns: Vec::new(),
            tool_calls: Vec::new(),
            tool_observations: Vec::new(),
            retrieval_notes: Vec::new(),
            responded_via: String::new(),
            final_confidence: None,
            open_questions: Vec::new(),
            claims: Vec::new(),
            rejected_claims: Vec::new(),
            unsupported_claim_count: 0,
            quote_span_coverage: None,
            citation_precision: None,
            stop_reason: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        let mut executed_tool_calls = 0usize;
        let tool_specs = registry.specs();

        loop {
            let turn_index = trace.provider_turns.len() + 1;
            let request = AgentInferenceRequest {
                model: ModelSelection {
                    provider: self.provider.clone(),
                    model: self.model.clone(),
                },
                instructions: InstructionBundle {
                    system: Some(agentic_retrieval_system_prompt().to_string()),
                    developer: None,
                },
                transcript: transcript.clone(),
                tools: tool_specs.clone(),
                tool_choice: ToolChoice::Auto,
                reasoning: ReasoningConfig {
                    enabled: self.reasoning_level.is_some(),
                    level: self.reasoning_level.clone(),
                },
                output: OutputConfig {
                    max_tokens: Some(self.config.max_tokens),
                    ..Default::default()
                },
                runtime: RuntimeHints {
                    profile: RuntimeProfile::Eval,
                    parallel_tool_calls: Some(self.config.parallel_tool_calls),
                    ..Default::default()
                },
                metadata: json!({
                    "mode": "gbrain_agentic_tools",
                    "requestMutationsAllowed": false,
                    "scopeId": trace.scope_id,
                    "asOf": trace.as_of,
                }),
            };
            let ctx = InferenceTurnContext {
                thread_id: "gbrain-agentic-tools",
                turn_id: "answer",
                tool_executor: None,
            };
            let mut stream = self.engine.stream_turn(ctx, request).await?;
            let mut message = String::new();
            let mut tool_calls = Vec::new();
            let mut stop_reason = None;
            let mut turn_usage = TokenUsage::default();

            while let Some(event) = stream.next().await {
                match event? {
                    InferenceEvent::MessageDelta(delta) => message.push_str(&delta.text),
                    InferenceEvent::ToolCallCompleted(call) => {
                        let arguments = parse_tool_arguments(&call.arguments);
                        let arguments = apply_default_scope(call.name.as_str(), arguments, &scope);
                        tool_calls.push(ModelSelectedToolCall::new(call.id, call.name, arguments));
                    }
                    InferenceEvent::Usage(usage) => turn_usage.add_assign(&usage),
                    InferenceEvent::Completed(metadata) => {
                        stop_reason = metadata.stop_reason;
                    }
                    InferenceEvent::Failed(failure) => {
                        anyhow::bail!("inference failed: {}", failure.message);
                    }
                    _ => {}
                }
            }

            trace.input_tokens = trace.input_tokens.saturating_add(turn_usage.prompt_tokens);
            trace.output_tokens = trace
                .output_tokens
                .saturating_add(turn_usage.completion_tokens);
            trace.provider_turns.push(AgenticProviderTurnTrace {
                turn_index,
                transcript_items: transcript.len(),
                message: message.clone(),
                tool_calls: tool_calls.clone(),
                stop_reason: stop_reason.clone(),
            });
            trace.stop_reason = stop_reason;

            if tool_calls.is_empty() {
                trace.responded_via = "final_text".to_string();
                if trace.stop_reason.is_none() {
                    trace.stop_reason = Some("final_text".to_string());
                }
                return Ok(AgenticToolAnswer {
                    answer: message,
                    provenance: Vec::new(),
                    trace,
                });
            }

            if !message.trim().is_empty() {
                transcript.push(TranscriptItem::AssistantMessage(AssistantMessage {
                    text: message,
                    phase: None,
                }));
            }

            let next_tool_call_count = executed_tool_calls + tool_calls.len();
            if self
                .config
                .max_tool_calls
                .is_some_and(|max_tool_calls| next_tool_call_count > max_tool_calls)
            {
                trace.responded_via = "safety_rail".to_string();
                trace.stop_reason = Some("max_tool_calls".to_string());
                return Ok(AgenticToolAnswer {
                    answer: String::new(),
                    provenance: Vec::new(),
                    trace,
                });
            }

            let first_tool_index = executed_tool_calls + 1;
            executed_tool_calls = next_tool_call_count;

            for call in &tool_calls {
                trace.tool_calls.push(call.clone());
                transcript.push(TranscriptItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.to_string(),
                }));
            }

            let results = if self.config.parallel_tool_calls {
                join_all(tool_calls.iter().enumerate().map(|(offset, call)| {
                    execute_agentic_tool(&registry, call, first_tool_index + offset)
                }))
                .await
            } else {
                let mut results = Vec::with_capacity(tool_calls.len());
                for (offset, call) in tool_calls.iter().enumerate() {
                    results.push(
                        execute_agentic_tool(&registry, call, first_tool_index + offset).await,
                    );
                }
                results
            };

            let mut batch_observations = Vec::with_capacity(tool_calls.len());
            for (call, result) in tool_calls.into_iter().zip(results) {
                let result = result?;
                let result_record = ToolResultRecord {
                    id: result.id.clone(),
                    name: Some(result.name.clone()),
                    result: result.text.clone(),
                    display_payload: tool_display_payload(
                        Some(&result.name),
                        Some(&call.arguments),
                        Some(&result.data),
                    ),
                    is_error: result.is_error,
                };
                transcript.push(TranscriptItem::ToolResult(result_record));
                let observation = ToolObservation {
                    call: call.clone(),
                    result,
                };
                trace.tool_observations.push(observation.clone());
                batch_observations.push(observation);
            }

            let mut final_answer = None;
            for observation in &batch_observations {
                capture_trace_fields(&mut trace, &observation.call, &observation.result);
                if observation.call.name == "respond_to_query"
                    && !observation.result.is_error
                    && final_answer.is_none()
                {
                    trace.responded_via = "respond_to_query".to_string();
                    trace.stop_reason = Some("respond_to_query".to_string());
                    final_answer = Some((
                        observation.result.text.clone(),
                        cited_evidence_ids(&observation.result.data),
                    ));
                }
            }
            if let Some((answer, provenance)) = final_answer {
                return Ok(AgenticToolAnswer {
                    answer,
                    provenance,
                    trace,
                });
            }
        }
    }
}
