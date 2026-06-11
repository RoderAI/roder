use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::extension::InferenceEngineId;
use crate::reliability::ReliabilityRequestPolicy;
use crate::tools::{ToolChoice, ToolSpec};
use crate::transcript::TranscriptItem;

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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchMode {
    #[default]
    Explicit,
    Auto,
    ProviderNative,
}

impl ToolSearchMode {
    pub fn allows_provider_native(self) -> bool {
        matches!(self, Self::Auto | Self::ProviderNative)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchProviderVariant {
    #[default]
    Default,
    Regex,
    Bm25,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchConfig {
    #[serde(default)]
    pub mode: ToolSearchMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_catalog_items: Option<u32>,
    #[serde(default)]
    pub include_mcp: bool,
    #[serde(default)]
    pub include_skills: bool,
    #[serde(default)]
    pub fallback_to_explicit_tools: bool,
    #[serde(default)]
    pub provider_variant: ToolSearchProviderVariant,
}

impl Default for ToolSearchConfig {
    fn default() -> Self {
        Self {
            mode: ToolSearchMode::Explicit,
            max_catalog_items: None,
            include_mcp: true,
            include_skills: true,
            fallback_to_explicit_tools: true,
            provider_variant: ToolSearchProviderVariant::Default,
        }
    }
}

impl ToolSearchConfig {
    pub fn explicit() -> Self {
        Self {
            mode: ToolSearchMode::Explicit,
            ..Self::default()
        }
    }

    pub fn provider_native() -> Self {
        Self {
            mode: ToolSearchMode::ProviderNative,
            ..Self::default()
        }
    }

    pub fn is_provider_native_requested(&self) -> bool {
        self.mode.allows_provider_native()
    }

    /**
     * Resolve the effective tool-search mode for one provider/model turn.
     *
     * `Auto` silently falls back to explicit tools when the provider/model
     * does not support native tool search. An explicit `ProviderNative`
     * request only falls back when `fallback_to_explicit_tools` allows it;
     * otherwise the turn must fail closed with the returned diagnostic.
     */
    pub fn resolve_effective_mode(
        &self,
        provider_native_supported: bool,
    ) -> Result<EffectiveToolSearchMode, ToolSearchModeError> {
        match self.mode {
            ToolSearchMode::Explicit => Ok(EffectiveToolSearchMode::Explicit),
            ToolSearchMode::Auto => {
                if provider_native_supported {
                    Ok(EffectiveToolSearchMode::ProviderNative)
                } else {
                    Ok(EffectiveToolSearchMode::Explicit)
                }
            }
            ToolSearchMode::ProviderNative => {
                if provider_native_supported {
                    Ok(EffectiveToolSearchMode::ProviderNative)
                } else if self.fallback_to_explicit_tools {
                    Ok(EffectiveToolSearchMode::Explicit)
                } else {
                    Err(ToolSearchModeError::ProviderNativeUnsupported)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveToolSearchMode {
    Explicit,
    ProviderNative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSearchModeError {
    ProviderNativeUnsupported,
}

impl std::fmt::Display for ToolSearchModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderNativeUnsupported => write!(
                f,
                "provider-native tool search was requested but the selected provider/model does \
                 not support it and fallback_to_explicit_tools is disabled; enable fallback or \
                 pick a supported model"
            ),
        }
    }
}

impl std::error::Error for ToolSearchModeError {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchConfigOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ToolSearchMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_catalog_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_mcp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_skills: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_to_explicit_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_variant: Option<ToolSearchProviderVariant>,
}

impl ToolSearchConfigOverlay {
    pub fn overlay(&mut self, other: &Self) {
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        if other.max_catalog_items.is_some() {
            self.max_catalog_items = other.max_catalog_items;
        }
        if other.include_mcp.is_some() {
            self.include_mcp = other.include_mcp;
        }
        if other.include_skills.is_some() {
            self.include_skills = other.include_skills;
        }
        if other.fallback_to_explicit_tools.is_some() {
            self.fallback_to_explicit_tools = other.fallback_to_explicit_tools;
        }
        if other.provider_variant.is_some() {
            self.provider_variant = other.provider_variant;
        }
    }

    pub fn apply_to(&self, config: &mut ToolSearchConfig) {
        if let Some(mode) = self.mode {
            config.mode = mode;
        }
        if let Some(max_catalog_items) = self.max_catalog_items {
            config.max_catalog_items = Some(max_catalog_items);
        }
        if let Some(include_mcp) = self.include_mcp {
            config.include_mcp = include_mcp;
        }
        if let Some(include_skills) = self.include_skills {
            config.include_skills = include_skills;
        }
        if let Some(fallback_to_explicit_tools) = self.fallback_to_explicit_tools {
            config.fallback_to_explicit_tools = fallback_to_explicit_tools;
        }
        if let Some(provider_variant) = self.provider_variant {
            config.provider_variant = provider_variant;
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
    Cursor,
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
    #[serde(default)]
    pub tool_search: ToolSearchConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_policy: Option<SpeedPolicyDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_remaining_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability: Option<ReliabilityRequestPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInferenceRequest {
    pub model: ModelSelection,
    pub instructions: InstructionBundle,
    pub transcript: Vec<TranscriptItem>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedToolCallStarted {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedToolCallCompleted {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub cached_prompt_tokens: u32,
    /**
     * Prompt tokens written to the provider's prompt cache this step. Like
     * `cached_prompt_tokens`, this is a subset of `prompt_tokens`, not an
     * additional count; hosts use it to bill cache writes at the provider's
     * cache-write rate.
     */
    #[serde(default)]
    pub cache_creation_prompt_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
}

impl TokenUsage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cached_prompt_tokens: 0,
            cache_creation_prompt_tokens: 0,
            cache_hit_rate: cache_hit_rate(prompt_tokens, 0),
        }
    }

    pub fn with_cached_prompt_tokens(mut self, cached_prompt_tokens: u32) -> Self {
        self.cached_prompt_tokens = cached_prompt_tokens.min(self.prompt_tokens);
        self.cache_hit_rate = cache_hit_rate(self.prompt_tokens, self.cached_prompt_tokens);
        self
    }

    pub fn with_cache_creation_prompt_tokens(mut self, cache_creation_prompt_tokens: u32) -> Self {
        self.cache_creation_prompt_tokens = cache_creation_prompt_tokens.min(self.prompt_tokens);
        self
    }

    pub fn add_assign(&mut self, usage: &TokenUsage) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(usage.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);
        self.cached_prompt_tokens = self
            .cached_prompt_tokens
            .saturating_add(usage.cached_prompt_tokens);
        self.cache_creation_prompt_tokens = self
            .cache_creation_prompt_tokens
            .saturating_add(usage.cache_creation_prompt_tokens);
        self.cache_hit_rate = cache_hit_rate(self.prompt_tokens, self.cached_prompt_tokens);
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.total_tokens == 0
            && self.cached_prompt_tokens == 0
            && self.cache_creation_prompt_tokens == 0
    }
}

pub fn cache_hit_rate(prompt_tokens: u32, cached_prompt_tokens: u32) -> Option<f64> {
    if prompt_tokens == 0 {
        None
    } else {
        Some(f64::from(cached_prompt_tokens.min(prompt_tokens)) / f64::from(prompt_tokens))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionMetadata {
    pub stop_reason: Option<String>,
    pub provider_response_id: Option<String>,
}

/**
 * Canonical mapping from provider-native stop reasons to the finish reason
 * surfaced as `finishReason` on `turn/completed`. Only the terminal inference
 * step's stop reason reaches the turn surface, so `toolUse` appears only when
 * a turn genuinely ends on a tool-use step (e.g. tool rounds exhausted).
 * Unknown stop reasons pass through unchanged.
 */
pub fn finish_reason_from_stop_reason(stop_reason: &str) -> String {
    match stop_reason {
        "end_turn" | "stop" | "stop_sequence" => "stop",
        "max_tokens" | "length" => "length",
        "tool_use" | "tool_calls" => "toolUse",
        "content_filter" => "contentFilter",
        "refusal" => "refusal",
        other => other,
    }
    .to_string()
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
    HostedToolCallStarted(HostedToolCallStarted),
    HostedToolCallCompleted(HostedToolCallCompleted),
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
    pub tool_search: bool,
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
            tool_search: false,
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
            tool_search: false,
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
    /// Optional callback that executes a single tool call through Roder's tool
    /// registry and policy, returning its result. Provided by the runtime for
    /// providers that drive their own in-stream agent loop (e.g. the Cursor
    /// bidi agent-runtime client, which must execute read/write/shell exec
    /// requests mid-stream rather than ending the turn). Most providers ignore
    /// it and surface tool calls as `ToolCallCompleted` events instead.
    pub tool_executor: Option<std::sync::Arc<dyn TurnToolExecutor>>,
}

/// Result of executing one tool call via [`TurnToolExecutor`].
#[derive(Debug, Clone)]
pub struct TurnToolOutcome {
    pub result: String,
    pub is_error: bool,
}

/// Executes a single tool call through the runtime's registry + policy.
/// Implemented by the runtime; used by providers that run their own in-stream
/// agent loop.
#[async_trait::async_trait]
pub trait TurnToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome>;
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
    fn finish_reason_mapping_normalizes_known_stop_reasons() {
        assert_eq!(finish_reason_from_stop_reason("end_turn"), "stop");
        assert_eq!(finish_reason_from_stop_reason("stop"), "stop");
        assert_eq!(finish_reason_from_stop_reason("stop_sequence"), "stop");
        assert_eq!(finish_reason_from_stop_reason("max_tokens"), "length");
        assert_eq!(finish_reason_from_stop_reason("length"), "length");
        assert_eq!(finish_reason_from_stop_reason("tool_use"), "toolUse");
        assert_eq!(finish_reason_from_stop_reason("tool_calls"), "toolUse");
        assert_eq!(
            finish_reason_from_stop_reason("content_filter"),
            "contentFilter"
        );
        assert_eq!(finish_reason_from_stop_reason("refusal"), "refusal");
        assert_eq!(finish_reason_from_stop_reason("pause_turn"), "pause_turn");
    }

    #[test]
    fn token_usage_accumulates_cache_creation_prompt_tokens() {
        let mut usage = TokenUsage::new(100, 10, 110)
            .with_cached_prompt_tokens(80)
            .with_cache_creation_prompt_tokens(15);
        usage.add_assign(
            &TokenUsage::new(50, 5, 55)
                .with_cached_prompt_tokens(40)
                .with_cache_creation_prompt_tokens(10),
        );

        assert_eq!(usage.prompt_tokens, 150);
        assert_eq!(usage.cached_prompt_tokens, 120);
        assert_eq!(usage.cache_creation_prompt_tokens, 25);
        assert!(!usage.is_empty());

        let creation_only = TokenUsage {
            cache_creation_prompt_tokens: 1,
            ..TokenUsage::default()
        };
        assert!(!creation_only.is_empty());
    }

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

    #[test]
    fn inference_reliability_policy_serializes_runtime_metadata() {
        let hints = RuntimeHints {
            reliability: Some(ReliabilityRequestPolicy::default()),
            ..RuntimeHints::default()
        };

        let json = serde_json::to_value(hints).unwrap();
        assert_eq!(
            json.get("reliability")
                .and_then(|value| value.get("providerRetryMaxAttempts"))
                .and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert_eq!(
            json.get("reliability")
                .and_then(|value| value.get("retryEmptyProviderBody"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn tool_search_config_serializes_provider_native_request() {
        let config = ToolSearchConfig {
            mode: ToolSearchMode::ProviderNative,
            max_catalog_items: Some(200),
            include_mcp: true,
            include_skills: false,
            fallback_to_explicit_tools: true,
            provider_variant: ToolSearchProviderVariant::Bm25,
        };

        let value = serde_json::to_value(&config).unwrap();

        assert_eq!(value["mode"], "provider_native");
        assert_eq!(value["maxCatalogItems"], 200);
        assert_eq!(value["includeMcp"], true);
        assert_eq!(value["includeSkills"], false);
        assert_eq!(value["providerVariant"], "bm25");
        assert!(config.is_provider_native_requested());
    }

    #[test]
    fn explicit_tool_search_config_preserves_current_default() {
        let config = ToolSearchConfig::default();

        assert_eq!(config.mode, ToolSearchMode::Explicit);
        assert!(!config.is_provider_native_requested());
        assert!(config.fallback_to_explicit_tools);
    }

    #[test]
    fn tool_search_effective_mode_resolution_covers_fallback_matrix() {
        let explicit = ToolSearchConfig::explicit();
        assert_eq!(
            explicit.resolve_effective_mode(true).unwrap(),
            EffectiveToolSearchMode::Explicit
        );

        let auto = ToolSearchConfig {
            mode: ToolSearchMode::Auto,
            ..ToolSearchConfig::default()
        };
        assert_eq!(
            auto.resolve_effective_mode(true).unwrap(),
            EffectiveToolSearchMode::ProviderNative
        );
        assert_eq!(
            auto.resolve_effective_mode(false).unwrap(),
            EffectiveToolSearchMode::Explicit
        );

        let native = ToolSearchConfig::provider_native();
        assert_eq!(
            native.resolve_effective_mode(true).unwrap(),
            EffectiveToolSearchMode::ProviderNative
        );
        assert_eq!(
            native.resolve_effective_mode(false).unwrap(),
            EffectiveToolSearchMode::Explicit
        );

        let strict = ToolSearchConfig {
            fallback_to_explicit_tools: false,
            ..ToolSearchConfig::provider_native()
        };
        let error = strict.resolve_effective_mode(false).unwrap_err();
        assert_eq!(error, ToolSearchModeError::ProviderNativeUnsupported);
        assert!(error.to_string().contains("fallback_to_explicit_tools"));
    }
}
