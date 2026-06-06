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

use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext, InstructionBundle,
    ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, RuntimeProfile,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};

use crate::reason::{Completion, Reasoner};

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
