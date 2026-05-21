use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::conversation::ConversationItem;
use crate::extension::InferenceEngineId;
use crate::tools::{ToolChoice, ToolSpec};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthType {
    None,
    ApiKey,
    OAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceProviderMetadata {
    pub name: String,
    pub description: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_label: Option<String>,
    pub auth_configured: Option<bool>,
    pub recommended: bool,
    pub sort_order: i32,
}

impl InferenceProviderMetadata {
    pub fn local(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            auth_type: ProviderAuthType::None,
            auth_label: None,
            auth_configured: Some(true),
            recommended: false,
            sort_order: 100,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstructionBundle {
    pub system: Option<String>,
    pub developer: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    #[default]
    Interactive,
    NonInteractive,
    Eval,
}

impl RuntimeProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::NonInteractive => "non_interactive",
            Self::Eval => "eval",
        }
    }

    pub fn is_non_interactive(self) -> bool {
        matches!(self, Self::NonInteractive | Self::Eval)
    }
}

impl std::str::FromStr for RuntimeProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "interactive" => Ok(Self::Interactive),
            "non_interactive" | "non-interactive" | "headless" => Ok(Self::NonInteractive),
            "eval" => Ok(Self::Eval),
            other => anyhow::bail!(
                "unsupported runtime profile {other:?}; expected interactive, non_interactive, or eval"
            ),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningConfig {
    pub enabled: bool,
    pub level: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFamily {
    #[default]
    Mock,
    OpenAi,
    Anthropic,
    Gemini,
    Xai,
    Opencode,
    Poolside,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelSchemaPolicy {
    #[default]
    StandardRequiredFirst,
    RequiredFirstFlat,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelInstructionOverlay {
    #[default]
    Standard,
    LiteralToolOutputs,
    IntuitiveContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileReasoning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelHarnessProfile {
    pub model: String,
    pub provider: String,
    pub provider_family: ProviderFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_tool: Option<String>,
    #[serde(default)]
    pub schema_policy: ModelSchemaPolicy,
    #[serde(default)]
    pub instruction_overlay: ModelInstructionOverlay,
    #[serde(default)]
    pub reasoning: ModelProfileReasoning,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeedPolicyPhase {
    #[default]
    Orientation,
    Execution,
    Verification,
    Recovery,
}

impl SpeedPolicyPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orientation => "orientation",
            Self::Execution => "execution",
            Self::Verification => "verification",
            Self::Recovery => "recovery",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeedPolicyDecision {
    pub phase: SpeedPolicyPhase,
    pub desired_reasoning: String,
    pub applied_reasoning: Option<String>,
    pub supported: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub response_format: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostedWebSearchMode {
    #[default]
    Disabled,
    Cached,
    Live,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedWebSearchConfig {
    pub mode: HostedWebSearchMode,
}

impl HostedWebSearchConfig {
    pub fn disabled() -> Self {
        Self {
            mode: HostedWebSearchMode::Disabled,
        }
    }

    pub fn cached() -> Self {
        Self {
            mode: HostedWebSearchMode::Cached,
        }
    }

    pub fn live() -> Self {
        Self {
            mode: HostedWebSearchMode::Live,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.mode != HostedWebSearchMode::Disabled
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeHints {
    pub trace_id: Option<String>,
    pub prompt_cache_key: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    #[serde(default)]
    pub profile: RuntimeProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub hosted_web_search: HostedWebSearchConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_policy: Option<SpeedPolicyDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_remaining_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInferenceRequest {
    pub model: ModelSelection,
    pub instructions: InstructionBundle,
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    pub reasoning: ReasoningConfig,
    pub output: OutputConfig,
    pub runtime: RuntimeHints,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageDelta {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningDelta {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallStarted {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallDelta {
    pub id: String,
    pub arguments_delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallCompleted {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionMetadata {
    pub stop_reason: Option<String>,
    pub provider_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceFailure {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactionProgress {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InferenceEvent {
    MessageDelta(MessageDelta),
    ReasoningDelta(ReasoningDelta),
    ToolCallStarted(ToolCallStarted),
    ToolCallDelta(ToolCallDelta),
    ToolCallCompleted(ToolCallCompleted),
    Compaction(CompactionProgress),
    Usage(TokenUsage),
    Completed(CompletionMetadata),
    Failed(InferenceFailure),
    ProviderMetadata(serde_json::Value),
}

pub type InferenceEventStream =
    Pin<Box<dyn Stream<Item = anyhow::Result<InferenceEvent>> + Send + 'static>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceCapabilities {
    pub streaming: bool,
    pub tool_calls: bool,
    pub parallel_tool_calls: bool,
    pub reasoning_summaries: bool,
    pub structured_output: bool,
    pub image_input: bool,
    pub prompt_cache: bool,
    pub provider_metadata: bool,
}

impl InferenceCapabilities {
    pub fn text_only() -> Self {
        Self {
            streaming: true,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: false,
            provider_metadata: false,
        }
    }

    pub fn coding_agent_default() -> Self {
        Self {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub name: String,
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_reasoning: Vec<ReasoningEffortDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningEffortDescriptor {
    pub effort: String,
    pub description: String,
}

pub struct InferenceProviderContext<'a> {
    pub provider_id: &'a str,
}

pub struct InferenceTurnContext<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

#[async_trait::async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    fn id(&self) -> InferenceEngineId;
    fn capabilities(&self) -> InferenceCapabilities;

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata::local(self.id())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_speed_policy_decision_serializes_runtime_metadata() {
        let decision = SpeedPolicyDecision {
            phase: SpeedPolicyPhase::Verification,
            desired_reasoning: "high".to_string(),
            applied_reasoning: Some("high".to_string()),
            supported: true,
        };
        let hints = RuntimeHints {
            speed_policy: Some(decision),
            ..RuntimeHints::default()
        };

        let json = serde_json::to_value(hints).unwrap();
        assert_eq!(
            json.get("speed_policy")
                .and_then(|value| value.get("phase"))
                .and_then(serde_json::Value::as_str),
            Some("verification")
        );
        assert_eq!(
            json.get("speed_policy")
                .and_then(|value| value.get("desiredReasoning"))
                .and_then(serde_json::Value::as_str),
            Some("high")
        );
        assert_eq!(
            json.get("speed_policy")
                .and_then(|value| value.get("appliedReasoning"))
                .and_then(serde_json::Value::as_str),
            Some("high")
        );
    }
}
