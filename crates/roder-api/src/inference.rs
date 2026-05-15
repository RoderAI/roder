use std::pin::Pin;
use futures::Stream;
use crate::conversation::ConversationItem;
use crate::tools::{ToolSpec, ToolChoice};
use serde::{Serialize, Deserialize};
use crate::extension::InferenceEngineId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionBundle {
    pub system: Option<String>,
    pub developer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    pub enabled: bool,
    pub level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHints {
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentInferenceRequest {
    pub model: ModelSelection,
    pub instructions: InstructionBundle,
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    pub reasoning: ReasoningConfig,
    pub output: OutputConfig,
    pub runtime: RuntimeHints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDelta {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningDelta {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStarted {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub id: String,
    pub arguments_delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompleted {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionMetadata {
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceFailure {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InferenceEvent {
    MessageDelta(MessageDelta),
    ReasoningDelta(ReasoningDelta),
    ToolCallStarted(ToolCallStarted),
    ToolCallDelta(ToolCallDelta),
    ToolCallCompleted(ToolCallCompleted),
    Usage(TokenUsage),
    Completed(CompletionMetadata),
    Failed(InferenceFailure),
}

pub type InferenceEventStream = Pin<Box<dyn Stream<Item = anyhow::Result<InferenceEvent>> + Send + 'static>>;

#[derive(Debug, Clone)]
pub struct InferenceCapabilities {
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
}

#[derive(Debug, Clone)]
pub struct ModelDescriptor {
    pub id: String,
    pub name: String,
}

pub struct InferenceProviderContext<'a> {
    pub provider_id: &'a str,
}

pub struct InferenceTurnContext<'a> {
    pub turn_id: &'a str,
}

#[async_trait::async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    fn id(&self) -> InferenceEngineId;
    fn capabilities(&self) -> InferenceCapabilities;

    async fn list_models(
        &self,
        ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>>;

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream>;
}
