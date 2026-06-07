use serde::Serialize;

use crate::inference::{
    ModelDescriptor, ModelHarnessProfile, ModelInstructionOverlay, ModelProfileReasoning,
    ModelSchemaPolicy, ProviderFamily, ReasoningEffortDescriptor,
};

mod xiaomi_mimo;

pub use xiaomi_mimo::{XIAOMI_MIMO_ENV_ALIASES, XIAOMI_MIMO_TOKEN_PLAN_ENV_ALIASES};

pub const PROVIDER_MOCK: &str = "mock";
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_CODEX: &str = "codex";
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_CLAUDE_CODE: &str = "claude-code";
pub const PROVIDER_GEMINI: &str = "gemini";
pub const PROVIDER_GOOGLE: &str = "google";
pub const PROVIDER_XAI: &str = "xai";
pub const PROVIDER_SUPERGROK: &str = "supergrok";
pub const PROVIDER_OPENCODE: &str = "opencode";
pub const PROVIDER_OPENCODE_GO: &str = "opencode-go";
pub const PROVIDER_OPENROUTER: &str = "openrouter";
pub const PROVIDER_POOLSIDE: &str = "poolside";
pub const PROVIDER_CURSOR: &str = "cursor";
pub const PROVIDER_XIAOMI_MIMO: &str = "xiaomi-mimo";
pub const PROVIDER_XIAOMI_MIMO_TOKEN_PLAN: &str = "xiaomi-mimo-token-plan";

pub const PROVIDER_KIND_MOCK: &str = "mock";
pub const PROVIDER_KIND_OPENAI: &str = "openai";
pub const PROVIDER_KIND_CHAT_COMPLETIONS: &str = "chat_completions";
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_KIND_CLAUDE_CODE: &str = "claude_code";
pub const PROVIDER_KIND_GEMINI: &str = "gemini";
pub const PROVIDER_KIND_XAI: &str = "xai";
pub const PROVIDER_KIND_OPENCODE: &str = "opencode";
pub const PROVIDER_KIND_OPENROUTER: &str = "openrouter";
pub const PROVIDER_KIND_POOLSIDE: &str = "poolside";
pub const PROVIDER_KIND_CURSOR: &str = "cursor";
pub const PROVIDER_KIND_XIAOMI_MIMO: &str = PROVIDER_KIND_CHAT_COMPLETIONS;

pub const REASONING_NONE: &str = "none";
pub const REASONING_MINIMAL: &str = "minimal";
pub const REASONING_LOW: &str = "low";
pub const REASONING_MEDIUM: &str = "medium";
pub const REASONING_HIGH: &str = "high";
pub const REASONING_XHIGH: &str = "xhigh";
pub const REASONING_MAX: &str = "max";

pub const DEFAULT_MODEL_ID: &str = "gpt-5.5";
pub const EDIT_TOOL_PATCH: &str = "patch";
pub const EDIT_TOOL_EDIT: &str = "edit";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub default_model: &'static str,
    pub base_url: Option<&'static str>,
    pub env_key: Option<&'static str>,
    pub env_aliases: &'static [&'static str],
    pub requires_auth: bool,
    pub supports_websockets: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReasoningOption {
    pub effort: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub provider: &'static str,
    pub default_reasoning: &'static str,
    pub supported_reasoning: &'static [ReasoningOption],
    pub context_window: u32,
    pub max_context_window: u32,
    pub auto_compact_token_limit: u32,
    pub supports_compaction: bool,
    pub supports_images: bool,
    pub supports_tools: bool,
    pub supports_structured: bool,
    pub edit_tool: Option<&'static str>,
    pub hidden: bool,
}

pub const STANDARD_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Fast responses with lighter reasoning",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Balances speed and reasoning depth for everyday tasks",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "Greater reasoning depth for complex problems",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extra high reasoning depth for complex problems",
    },
];

// Claude Opus 4.7/4.8 support the full effort range, including `xhigh` for
// long-horizon agentic work and `max` for genuinely frontier problems.
pub const OPUS_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Most efficient; best for short, scoped tasks",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Balanced reasoning depth for cost-sensitive workflows",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High capability for complex reasoning and agentic tasks",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extended capability for long-horizon coding and agentic work",
    },
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Absolute maximum capability with no constraints on token spending",
    },
];

// Claude Sonnet 4.6 supports `max` but not `xhigh`.
pub const SONNET_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Most efficient; lowest latency and cost",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Balances speed, cost, and performance for most tasks",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "Greater reasoning depth for complex problems",
    },
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Absolute maximum capability with no constraints on token spending",
    },
];

pub const GPT_52_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Balances speed with some reasoning; useful for straightforward queries and short explanations",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Provides a solid balance of reasoning depth and latency for general-purpose tasks",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "Maximizes reasoning depth for complex or ambiguous problems",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extra high reasoning for complex problems",
    },
];

pub const HAIKU_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Fast responses with lighter reasoning",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Balances speed and reasoning depth for everyday tasks",
    },
];

pub const GEMINI_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_MINIMAL,
        description: "Minimal Gemini thinking",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low Gemini thinking",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Medium Gemini thinking",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High Gemini thinking",
    },
];

pub const MOCK_REASONING: &[ReasoningOption] = &[ReasoningOption {
    effort: REASONING_NONE,
    description: "No model-side reasoning",
}];

pub const POOLSIDE_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "Disable Poolside thinking for lower latency",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Enable Poolside thinking",
    },
];

pub const GEMINI_ENV_ALIASES: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_API_KEY",
    "GOOGLE_AI_API_KEY",
];

pub const XAI_ENV_ALIASES: &[&str] = &["RODER_XAI_API_KEY"];

pub const XAI_CONFIGURABLE_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "No xAI reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low xAI reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Medium xAI reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High xAI reasoning effort",
    },
];

pub const XAI_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low xAI reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Medium xAI reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High xAI reasoning effort",
    },
];

pub const XAI_NO_REASONING: &[ReasoningOption] = &[ReasoningOption {
    effort: REASONING_NONE,
    description: "No xAI reasoning effort",
}];

pub const OPENROUTER_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "Disable OpenRouter reasoning controls",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low OpenRouter reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Medium OpenRouter reasoning effort",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High OpenRouter reasoning effort",
    },
];

pub const BUILT_IN_PROVIDERS: &[ProviderCatalogEntry] = &[
    ProviderCatalogEntry {
        id: PROVIDER_MOCK,
        name: "Mock",
        kind: PROVIDER_KIND_MOCK,
        default_model: "mock",
        base_url: None,
        env_key: None,
        env_aliases: &[],
        requires_auth: false,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_OPENAI,
        name: "OpenAI",
        kind: PROVIDER_KIND_OPENAI,
        default_model: DEFAULT_MODEL_ID,
        base_url: Some("https://api.openai.com/v1"),
        env_key: Some("OPENAI_API_KEY"),
        env_aliases: &[],
        requires_auth: true,
        supports_websockets: true,
    },
    ProviderCatalogEntry {
        id: PROVIDER_CODEX,
        name: "Codex",
        kind: PROVIDER_KIND_OPENAI,
        default_model: DEFAULT_MODEL_ID,
        base_url: Some("https://api.openai.com/v1"),
        env_key: Some("OPENAI_API_KEY"),
        env_aliases: &[],
        requires_auth: true,
        supports_websockets: true,
    },
    ProviderCatalogEntry {
        id: PROVIDER_ANTHROPIC,
        name: "Anthropic",
        kind: PROVIDER_KIND_ANTHROPIC,
        default_model: "claude-sonnet-4-6",
        base_url: Some("https://api.anthropic.com"),
        env_key: Some("ANTHROPIC_API_KEY"),
        env_aliases: &[],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_CLAUDE_CODE,
        name: "Claude Code",
        kind: PROVIDER_KIND_CLAUDE_CODE,
        default_model: "sonnet",
        base_url: None,
        env_key: None,
        env_aliases: &["CLAUDE_CODE_CLI_PATH", "RODER_CLAUDE_CODE_CLI_PATH"],
        requires_auth: false,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_GEMINI,
        name: "Gemini",
        kind: PROVIDER_KIND_GEMINI,
        default_model: "gemini-3.5-flash",
        base_url: None,
        env_key: Some("GEMINI_API_TOKEN"),
        env_aliases: GEMINI_ENV_ALIASES,
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_XAI,
        name: "xAI",
        kind: PROVIDER_KIND_XAI,
        default_model: "grok-4.3",
        base_url: Some("https://api.x.ai/v1"),
        env_key: Some("XAI_API_KEY"),
        env_aliases: XAI_ENV_ALIASES,
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_SUPERGROK,
        name: "SuperGrok",
        kind: PROVIDER_KIND_XAI,
        default_model: "grok-4.3",
        base_url: Some("https://api.x.ai/v1"),
        env_key: None,
        env_aliases: &[],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_OPENCODE,
        name: "OpenCode Zen",
        kind: PROVIDER_KIND_OPENCODE,
        default_model: "gpt-5.5",
        base_url: Some("https://opencode.ai/zen/v1"),
        env_key: Some("OPENCODE_API_KEY"),
        env_aliases: &["OPENCODE_ZEN_API_KEY", "RODER_OPENCODE_API_KEY"],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_OPENCODE_GO,
        name: "OpenCode Go",
        kind: PROVIDER_KIND_OPENCODE,
        default_model: "kimi-k2.6",
        base_url: Some("https://opencode.ai/zen/go/v1"),
        env_key: Some("OPENCODE_GO_API_KEY"),
        env_aliases: &["RODER_OPENCODE_GO_API_KEY", "OPENCODE_API_KEY"],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_OPENROUTER,
        name: "OpenRouter",
        kind: PROVIDER_KIND_OPENROUTER,
        default_model: "x-ai/grok-build-0.1",
        base_url: Some("https://openrouter.ai/api/v1"),
        env_key: Some("OPENROUTER_API_KEY"),
        env_aliases: &["RODER_OPENROUTER_API_KEY"],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_POOLSIDE,
        name: "Poolside",
        kind: PROVIDER_KIND_POOLSIDE,
        default_model: "poolside/laguna-m.1",
        base_url: Some("https://inference.poolside.ai/v1"),
        env_key: Some("POOLSIDE_API_KEY"),
        env_aliases: &["RODER_POOLSIDE_API_KEY"],
        requires_auth: true,
        supports_websockets: false,
    },
    ProviderCatalogEntry {
        id: PROVIDER_CURSOR,
        name: "Cursor",
        kind: PROVIDER_KIND_CURSOR,
        default_model: "composer-2.5",
        base_url: Some("https://agentn.global.api5.cursor.sh"),
        env_key: Some("CURSOR_API_KEY"),
        env_aliases: &["RODER_CURSOR_API_KEY"],
        requires_auth: true,
        supports_websockets: false,
    },
    xiaomi_mimo::PAY_AS_YOU_GO_PROVIDER,
    xiaomi_mimo::TOKEN_PLAN_PROVIDER,
];

pub const BUILT_IN_MODELS: &[ModelCatalogEntry] = &[
    openai_model(
        "gpt-5.5",
        "GPT-5.5",
        "Frontier model for complex coding, research, and real-world work.",
        1_050_000,
        945_000,
        true,
        STANDARD_REASONING,
    ),
    openai_model(
        "gpt-5.4-mini",
        "GPT-5.4-Mini",
        "Small, fast, and cost-efficient model for simpler coding tasks.",
        400_000,
        360_000,
        true,
        STANDARD_REASONING,
    ),
    ModelCatalogEntry {
        id: "gpt-5.3-codex-spark",
        display_name: "GPT-5.3-Codex-Spark",
        description: "Ultra-fast coding model optimized for low-latency Codex workflows.",
        provider: PROVIDER_CODEX,
        default_reasoning: REASONING_HIGH,
        supported_reasoning: STANDARD_REASONING,
        context_window: 128_000,
        max_context_window: 128_000,
        auto_compact_token_limit: 115_200,
        supports_compaction: true,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some("patch"),
        hidden: false,
    },
    ModelCatalogEntry {
        id: "codex-auto-review",
        display_name: "Codex Auto Review",
        description: "Automatic approval review model for Codex.",
        provider: PROVIDER_OPENAI,
        default_reasoning: REASONING_MEDIUM,
        supported_reasoning: STANDARD_REASONING,
        context_window: 272_000,
        max_context_window: 272_000,
        auto_compact_token_limit: 244_800,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some("patch"),
        hidden: true,
    },
    anthropic_model(
        "claude-opus-4-8",
        "Claude Opus 4.8",
        "Anthropic's most capable model for complex reasoning, long-horizon agentic coding, and high-autonomy work.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        OPUS_REASONING,
    ),
    anthropic_model(
        "claude-opus-4-7",
        "Claude Opus 4.7",
        "Most capable Claude model for complex reasoning and agentic coding.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        OPUS_REASONING,
    ),
    anthropic_model(
        "claude-sonnet-4-6",
        "Claude Sonnet 4.6",
        "Balanced Claude model for coding, tool use, and everyday agent workflows.",
        1_000_000,
        900_000,
        REASONING_MEDIUM,
        SONNET_REASONING,
    ),
    anthropic_model(
        "claude-haiku-4-5-20251001",
        "Claude Haiku 4.5",
        "Fast Claude model for lower-latency tool workflows.",
        200_000,
        180_000,
        REASONING_NONE,
        &[],
    ),
    claude_code_model(
        "sonnet",
        "Claude Code Sonnet",
        "Claude Code harness Sonnet alias for coding and tool workflows.",
        1_000_000,
        900_000,
        REASONING_MEDIUM,
        SONNET_REASONING,
    ),
    claude_code_model(
        "opus",
        "Claude Code Opus",
        "Claude Code harness Opus alias for complex long-horizon agentic work.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        OPUS_REASONING,
    ),
    claude_code_model(
        "haiku",
        "Claude Code Haiku",
        "Claude Code harness Haiku alias for fast lower-latency coding turns.",
        200_000,
        180_000,
        REASONING_NONE,
        &[],
    ),
    claude_code_model(
        "claude-sonnet-4-6",
        "Claude Code Sonnet 4.6",
        "Claude Sonnet 4.6 through the local Claude Code harness.",
        1_000_000,
        900_000,
        REASONING_MEDIUM,
        SONNET_REASONING,
    ),
    claude_code_model(
        "claude-opus-4-8",
        "Claude Code Opus 4.8",
        "Claude Opus 4.8 through the local Claude Code harness.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        OPUS_REASONING,
    ),
    gemini_model(
        "gemini-3.5-flash",
        "Gemini 3.5 Flash",
        "Stable Gemini Flash model for agentic coding, tool use, and long-horizon workflows.",
        REASONING_MEDIUM,
    ),
    gemini_model(
        "gemini-3.1-pro-preview",
        "Gemini 3.1 Pro Preview",
        "Gemini model for complex coding, long context, and tool-heavy agent workflows.",
        REASONING_HIGH,
    ),
    gemini_model(
        "gemini-3.1-pro-preview-customtools",
        "Gemini 3.1 Pro Preview Custom Tools",
        "Gemini preview variant exposed for custom tool validation and tool-heavy coding workflows.",
        REASONING_HIGH,
    ),
    gemini_model(
        "gemini-3-flash-preview",
        "Gemini 3 Flash Preview",
        "Fast Gemini model for everyday coding, tool use, and multimodal prompts.",
        REASONING_MEDIUM,
    ),
    gemini_model(
        "gemini-3.1-flash-lite-preview",
        "Gemini 3.1 Flash-Lite Preview",
        "Lightweight Gemini model for low-latency coding and agent interactions.",
        REASONING_LOW,
    ),
    xai_model(
        PROVIDER_XAI,
        "grok-4.3",
        "Grok 4.3",
        "xAI flagship model for chat, coding, tool use, and configurable reasoning.",
        1_000_000,
        REASONING_LOW,
        XAI_CONFIGURABLE_REASONING,
    ),
    xai_model(
        PROVIDER_XAI,
        "grok-4.20-multi-agent-0309",
        "Grok 4.20 Multi-Agent",
        "xAI long-context model with agentic tool-calling and reasoning.",
        2_000_000,
        REASONING_LOW,
        XAI_REASONING,
    ),
    xai_model(
        PROVIDER_XAI,
        "grok-4.20-0309-reasoning",
        "Grok 4.20 Reasoning",
        "xAI long-context reasoning model for complex tool-heavy workflows.",
        2_000_000,
        REASONING_LOW,
        XAI_REASONING,
    ),
    xai_model(
        PROVIDER_XAI,
        "grok-4.20-0309-non-reasoning",
        "Grok 4.20 Non-Reasoning",
        "xAI long-context model for lower-latency non-reasoning workflows.",
        2_000_000,
        REASONING_NONE,
        XAI_NO_REASONING,
    ),
    xai_model(
        PROVIDER_SUPERGROK,
        "grok-4.3",
        "Grok 4.3",
        "SuperGrok OAuth access to xAI Grok 4.3.",
        1_000_000,
        REASONING_LOW,
        XAI_CONFIGURABLE_REASONING,
    ),
    xai_model(
        PROVIDER_SUPERGROK,
        "grok-4.20-multi-agent-0309",
        "Grok 4.20 Multi-Agent",
        "SuperGrok OAuth access to xAI's long-context multi-agent model.",
        2_000_000,
        REASONING_LOW,
        XAI_REASONING,
    ),
    xai_model(
        PROVIDER_SUPERGROK,
        "grok-4.20-0309-reasoning",
        "Grok 4.20 Reasoning",
        "SuperGrok OAuth access to xAI's long-context reasoning model.",
        2_000_000,
        REASONING_LOW,
        XAI_REASONING,
    ),
    xai_model(
        PROVIDER_SUPERGROK,
        "grok-4.20-0309-non-reasoning",
        "Grok 4.20 Non-Reasoning",
        "SuperGrok OAuth access to xAI's long-context non-reasoning model.",
        2_000_000,
        REASONING_NONE,
        XAI_NO_REASONING,
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "gpt-5.5",
        "GPT 5.5",
        "OpenCode Zen GPT 5.5 gateway model.",
        1_050_000,
        REASONING_MEDIUM,
        STANDARD_REASONING,
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "gpt-5.3-codex-spark",
        "GPT 5.3 Codex Spark",
        "OpenCode Zen low-latency Codex model.",
        128_000,
        REASONING_HIGH,
        STANDARD_REASONING,
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "big-pickle",
        "Big Pickle",
        "OpenCode Zen free coding model.",
        256_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "deepseek-v4-flash-free",
        "DeepSeek V4 Flash Free",
        "OpenCode Zen free DeepSeek coding model.",
        128_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "minimax-m2.5-free",
        "MiniMax M2.5 Free",
        "OpenCode Zen free MiniMax coding model.",
        256_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE,
        "nemotron-3-super-free",
        "Nemotron 3 Super Free",
        "OpenCode Zen free Nemotron coding model.",
        128_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE_GO,
        "kimi-k2.6",
        "Kimi K2.6",
        "OpenCode Go Kimi coding model.",
        256_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE_GO,
        "qwen3.6-plus",
        "Qwen3.6 Plus",
        "OpenCode Go Qwen coding model.",
        256_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE_GO,
        "glm-5.1",
        "GLM-5.1",
        "OpenCode Go GLM coding model.",
        256_000,
        REASONING_NONE,
        &[],
    ),
    opencode_model(
        PROVIDER_OPENCODE_GO,
        "deepseek-v4-flash",
        "DeepSeek V4 Flash",
        "OpenCode Go DeepSeek coding model.",
        128_000,
        REASONING_NONE,
        &[],
    ),
    ModelCatalogEntry {
        id: "x-ai/grok-build-0.1",
        display_name: "Grok Build 0.1",
        description: "OpenRouter route for xAI's fast coding model for agentic software engineering workflows.",
        provider: PROVIDER_OPENROUTER,
        default_reasoning: REASONING_LOW,
        supported_reasoning: OPENROUTER_REASONING,
        context_window: 256_000,
        max_context_window: 256_000,
        auto_compact_token_limit: 230_400,
        supports_compaction: true,
        supports_images: true,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some(EDIT_TOOL_PATCH),
        hidden: false,
    },
    poolside_model(
        "poolside/laguna-m.1",
        "Laguna M.1",
        "Poolside flagship agentic coding model.",
        REASONING_MEDIUM,
    ),
    poolside_model(
        "poolside/laguna-xs.2",
        "Laguna XS.2",
        "Poolside lightweight agentic coding model.",
        REASONING_MEDIUM,
    ),
    xiaomi_mimo::PAYG_V25_PRO,
    xiaomi_mimo::PAYG_V2_PRO,
    xiaomi_mimo::PAYG_V25,
    xiaomi_mimo::PAYG_V2_OMNI,
    xiaomi_mimo::PAYG_V2_FLASH,
    xiaomi_mimo::TOKEN_PLAN_V25_PRO,
    xiaomi_mimo::TOKEN_PLAN_V2_PRO,
    xiaomi_mimo::TOKEN_PLAN_V25,
    xiaomi_mimo::TOKEN_PLAN_V2_OMNI,
    xiaomi_mimo::TOKEN_PLAN_V2_FLASH,
    ModelCatalogEntry {
        id: "composer-2.5",
        display_name: "Composer 2.5",
        description: "Cursor Composer model exposed through direct AgentService inference.",
        provider: PROVIDER_CURSOR,
        default_reasoning: REASONING_NONE,
        supported_reasoning: &[],
        context_window: 200_000,
        max_context_window: 200_000,
        auto_compact_token_limit: 180_000,
        supports_compaction: true,
        supports_images: false,
        supports_tools: false,
        supports_structured: false,
        edit_tool: None,
        hidden: false,
    },
    cursor_model(
        "claude-opus-4-8",
        "Claude Opus 4.8",
        "Anthropic Claude Opus 4.8 routed through Cursor's AgentService.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        OPUS_REASONING,
    ),
    cursor_model(
        "claude-sonnet-4-6",
        "Claude Sonnet 4.6",
        "Anthropic Claude Sonnet 4.6 routed through Cursor's AgentService.",
        1_000_000,
        900_000,
        REASONING_NONE,
        &[],
    ),
    cursor_model(
        "gpt-5.5",
        "GPT-5.5",
        "OpenAI GPT-5.5 routed through Cursor's AgentService.",
        1_050_000,
        945_000,
        REASONING_NONE,
        &[],
    ),
    cursor_model(
        "gemini-3.1-pro-preview",
        "Gemini 3.1 Pro",
        "Google Gemini 3.1 Pro routed through Cursor's AgentService.",
        1_048_576,
        943_718,
        REASONING_NONE,
        &[],
    ),
    cursor_model(
        "grok-4.3",
        "Grok 4.3",
        "xAI Grok 4.3 routed through Cursor's AgentService.",
        1_000_000,
        900_000,
        REASONING_NONE,
        &[],
    ),
    ModelCatalogEntry {
        id: "text-embedding-3-large",
        display_name: "Text Embedding 3 Large",
        description: "OpenAI embedding model for local semantic memories.",
        provider: PROVIDER_OPENAI,
        default_reasoning: REASONING_NONE,
        supported_reasoning: &[],
        context_window: 0,
        max_context_window: 0,
        auto_compact_token_limit: 0,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: None,
        hidden: true,
    },
    ModelCatalogEntry {
        id: "gemini-embedding-2",
        display_name: "Gemini Embedding 2",
        description: "Google Gemini embedding model for local semantic memories.",
        provider: PROVIDER_GOOGLE,
        default_reasoning: REASONING_NONE,
        supported_reasoning: &[],
        context_window: 0,
        max_context_window: 0,
        auto_compact_token_limit: 0,
        supports_compaction: false,
        supports_images: false,
        supports_tools: false,
        supports_structured: false,
        edit_tool: None,
        hidden: true,
    },
    ModelCatalogEntry {
        id: "mock",
        display_name: "Mock",
        description: "Local deterministic mock provider for tests and offline development.",
        provider: PROVIDER_MOCK,
        default_reasoning: REASONING_NONE,
        supported_reasoning: MOCK_REASONING,
        context_window: 128_000,
        max_context_window: 128_000,
        auto_compact_token_limit: 115_200,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: None,
        hidden: true,
    },
];

const fn openai_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    auto_compact_token_limit: u32,
    supports_compaction: bool,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_OPENAI,
        default_reasoning: REASONING_MEDIUM,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit,
        supports_compaction,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some("patch"),
        hidden: false,
    }
}

const fn anthropic_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    auto_compact_token_limit: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_ANTHROPIC,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some("edit"),
        hidden: false,
    }
}

const fn claude_code_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    auto_compact_token_limit: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_CLAUDE_CODE,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit,
        supports_compaction: true,
        supports_images: false,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some(EDIT_TOOL_EDIT),
        hidden: false,
    }
}

const fn gemini_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    default_reasoning: &'static str,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_GEMINI,
        default_reasoning,
        supported_reasoning: GEMINI_REASONING,
        context_window: 1_048_576,
        max_context_window: 1_048_576,
        auto_compact_token_limit: 943_718,
        supports_compaction: false,
        supports_images: true,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some("edit"),
        hidden: false,
    }
}

const fn xai_model(
    provider: &'static str,
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit: context_window.saturating_mul(9) / 10,
        supports_compaction: false,
        supports_images: true,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some("edit"),
        hidden: false,
    }
}

const fn opencode_model(
    provider: &'static str,
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit: context_window.saturating_mul(9) / 10,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some("edit"),
        hidden: false,
    }
}

const fn poolside_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    default_reasoning: &'static str,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_POOLSIDE,
        default_reasoning,
        supported_reasoning: POOLSIDE_REASONING,
        context_window: 131_072,
        max_context_window: 131_072,
        auto_compact_token_limit: 117_964,
        supports_compaction: false,
        supports_images: false,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some("edit"),
        hidden: false,
    }
}

const fn cursor_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    auto_compact_token_limit: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_CURSOR,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit,
        supports_compaction: true,
        supports_images: false,
        supports_tools: false,
        supports_structured: false,
        edit_tool: None,
        hidden: false,
    }
}

pub fn built_in_providers() -> &'static [ProviderCatalogEntry] {
    BUILT_IN_PROVIDERS
}

pub fn built_in_models(include_hidden: bool) -> Vec<&'static ModelCatalogEntry> {
    BUILT_IN_MODELS
        .iter()
        .filter(|model| include_hidden || !model.hidden)
        .collect()
}

pub fn models_for_provider(provider: &str, include_hidden: bool) -> Vec<ModelDescriptor> {
    built_in_models(include_hidden)
        .into_iter()
        .filter(|model| model.provider == provider)
        .map(ModelDescriptor::from)
        .collect()
}

pub fn models_for_codex(include_hidden: bool) -> Vec<ModelDescriptor> {
    built_in_models(include_hidden)
        .into_iter()
        .filter(|model| model.provider == PROVIDER_OPENAI || model.provider == PROVIDER_CODEX)
        .map(ModelDescriptor::from)
        .collect()
}

pub fn lookup_model(id: &str) -> Option<&'static ModelCatalogEntry> {
    BUILT_IN_MODELS.iter().find(|model| model.id == id)
}

/// Resolve a catalog entry preferring an exact `(provider, id)` match.
///
/// Several model ids are shared across providers (for example `gpt-5.5` is
/// offered by both OpenAI and Cursor). [`lookup_model`] returns the first entry
/// by id, which silently resolves cross-provider ids to the wrong provider's
/// metadata. When the active provider is known, prefer this function so that,
/// e.g., `cursor/claude-opus-4-8` resolves to the Cursor catalog entry rather
/// than the Anthropic one. Falls back to id-only lookup so provider aliases and
/// user-defined models keep working.
pub fn lookup_model_for_provider(provider: &str, id: &str) -> Option<&'static ModelCatalogEntry> {
    BUILT_IN_MODELS
        .iter()
        .find(|model| model.provider == provider && model.id == id)
        .or_else(|| lookup_model(id))
}

pub fn built_in_model_profile(id: &str) -> Option<ModelHarnessProfile> {
    lookup_model(id).map(model_harness_profile_from_catalog)
}

/// Provider-aware variant of [`built_in_model_profile`].
///
/// Resolves the harness profile (provider family, instruction overlay, schema
/// policy, edit tool) using the active provider so cross-provider model ids
/// pick up the correct family instead of the first id match.
pub fn built_in_model_profile_for_provider(
    provider: &str,
    id: &str,
) -> Option<ModelHarnessProfile> {
    lookup_model_for_provider(provider, id).map(model_harness_profile_from_catalog)
}

pub fn built_in_model_profiles() -> Vec<ModelHarnessProfile> {
    built_in_models(true)
        .into_iter()
        .map(model_harness_profile_from_catalog)
        .collect()
}

fn model_harness_profile_from_catalog(model: &ModelCatalogEntry) -> ModelHarnessProfile {
    let provider_family = provider_family_for_provider(model.provider);
    ModelHarnessProfile {
        model: model.id.to_string(),
        provider: model.provider.to_string(),
        provider_family,
        edit_tool: model.edit_tool.map(str::to_string),
        schema_policy: schema_policy_for_family(provider_family),
        instruction_overlay: instruction_overlay_for_family(provider_family),
        reasoning: ModelProfileReasoning {
            orientation: Some(model.default_reasoning.to_string()),
            execution: Some(default_execution_reasoning(model)),
            verification: Some(model.default_reasoning.to_string()),
            recovery: Some(model.default_reasoning.to_string()),
        },
        parallel_tool_calls: Some(matches!(
            provider_family,
            ProviderFamily::OpenAi | ProviderFamily::Xai | ProviderFamily::Opencode
        )),
        auto_compact_token_limit: (model.auto_compact_token_limit > 0)
            .then_some(model.auto_compact_token_limit),
    }
}

pub fn provider_family_for_provider(provider: &str) -> ProviderFamily {
    match provider {
        PROVIDER_OPENAI | PROVIDER_CODEX => ProviderFamily::OpenAi,
        PROVIDER_ANTHROPIC | PROVIDER_CLAUDE_CODE => ProviderFamily::Anthropic,
        PROVIDER_GEMINI => ProviderFamily::Gemini,
        PROVIDER_XAI | PROVIDER_SUPERGROK => ProviderFamily::Xai,
        PROVIDER_OPENCODE | PROVIDER_OPENCODE_GO => ProviderFamily::Opencode,
        PROVIDER_OPENROUTER => ProviderFamily::OpenAi,
        PROVIDER_POOLSIDE => ProviderFamily::Poolside,
        PROVIDER_CURSOR => ProviderFamily::Cursor,
        PROVIDER_XIAOMI_MIMO | PROVIDER_XIAOMI_MIMO_TOKEN_PLAN => ProviderFamily::OpenAi,
        _ => ProviderFamily::Mock,
    }
}

fn schema_policy_for_family(family: ProviderFamily) -> ModelSchemaPolicy {
    match family {
        ProviderFamily::OpenAi => ModelSchemaPolicy::RequiredFirstFlat,
        _ => ModelSchemaPolicy::StandardRequiredFirst,
    }
}

fn instruction_overlay_for_family(family: ProviderFamily) -> ModelInstructionOverlay {
    match family {
        ProviderFamily::OpenAi => ModelInstructionOverlay::LiteralToolOutputs,
        ProviderFamily::Anthropic | ProviderFamily::Gemini => {
            ModelInstructionOverlay::IntuitiveContext
        }
        _ => ModelInstructionOverlay::Standard,
    }
}

fn default_execution_reasoning(model: &ModelCatalogEntry) -> String {
    if model
        .supported_reasoning
        .iter()
        .any(|option| option.effort == REASONING_LOW)
    {
        REASONING_LOW.to_string()
    } else {
        model.default_reasoning.to_string()
    }
}

pub fn model_supports_reasoning_effort(model: &str, effort: &str) -> bool {
    lookup_model(model)
        .map(|entry| {
            entry
                .supported_reasoning
                .iter()
                .any(|option| option.effort == effort)
        })
        .unwrap_or(false)
}

pub fn normalize_provider_id(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "grok" | "x-ai" | "x.ai" => PROVIDER_XAI.to_string(),
        "grok-oauth" | "xai-oauth" | "x-ai-oauth" | "xai-grok-oauth" => {
            PROVIDER_SUPERGROK.to_string()
        }
        "opencode" => PROVIDER_OPENCODE.to_string(),
        "go" | "opencode_go" | "opencode-go" => PROVIDER_OPENCODE_GO.to_string(),
        "openrouter" => PROVIDER_OPENROUTER.to_string(),
        "laguna" | "poolside" => PROVIDER_POOLSIDE.to_string(),
        "composer" | "cursor-composer" => PROVIDER_CURSOR.to_string(),
        "claude_code" | "claudecode" => PROVIDER_CLAUDE_CODE.to_string(),
        provider => provider.to_string(),
    }
}

impl From<&ModelCatalogEntry> for ModelDescriptor {
    fn from(model: &ModelCatalogEntry) -> Self {
        let supported_reasoning = model
            .supported_reasoning
            .iter()
            .map(|option| ReasoningEffortDescriptor {
                effort: option.effort.to_string(),
                description: option.description.to_string(),
            })
            .collect::<Vec<_>>();
        Self {
            id: model.id.to_string(),
            name: model.display_name.to_string(),
            context_window: (model.context_window > 0).then_some(model.context_window),
            default_reasoning: (!supported_reasoning.is_empty())
                .then(|| model.default_reasoning.to_string()),
            supported_reasoning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_gode_providers() {
        let ids = BUILT_IN_PROVIDERS
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "mock",
                "openai",
                "codex",
                "anthropic",
                "claude-code",
                "gemini",
                "xai",
                "supergrok",
                "opencode",
                "opencode-go",
                "openrouter",
                "poolside",
                "cursor",
                "xiaomi-mimo",
                "xiaomi-mimo-token-plan"
            ]
        );
    }

    #[test]
    fn gemini_provider_defaults_to_stable_35_flash() {
        let provider = BUILT_IN_PROVIDERS
            .iter()
            .find(|provider| provider.id == PROVIDER_GEMINI)
            .unwrap();

        assert_eq!(provider.default_model, "gemini-3.5-flash");

        let model = lookup_model("gemini-3.5-flash").unwrap();
        assert_eq!(model.display_name, "Gemini 3.5 Flash");
        assert_eq!(model.provider, PROVIDER_GEMINI);
        assert_eq!(model.context_window, 1_048_576);
        assert_eq!(model.default_reasoning, REASONING_MEDIUM);
        assert!(model.supports_tools);
        assert!(model.supports_structured);
        assert_eq!(
            model
                .supported_reasoning
                .iter()
                .map(|option| option.effort)
                .collect::<Vec<_>>(),
            vec![
                REASONING_MINIMAL,
                REASONING_LOW,
                REASONING_MEDIUM,
                REASONING_HIGH
            ]
        );
    }

    #[test]
    fn catalog_contains_gode_visible_models() {
        let ids = built_in_models(false)
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "gpt-5.5",
                "gpt-5.4-mini",
                "gpt-5.3-codex-spark",
                "claude-opus-4-8",
                "claude-opus-4-7",
                "claude-sonnet-4-6",
                "claude-haiku-4-5-20251001",
                "sonnet",
                "opus",
                "haiku",
                "claude-sonnet-4-6",
                "claude-opus-4-8",
                "gemini-3.5-flash",
                "gemini-3.1-pro-preview",
                "gemini-3.1-pro-preview-customtools",
                "gemini-3-flash-preview",
                "gemini-3.1-flash-lite-preview",
                "grok-4.3",
                "grok-4.20-multi-agent-0309",
                "grok-4.20-0309-reasoning",
                "grok-4.20-0309-non-reasoning",
                "grok-4.3",
                "grok-4.20-multi-agent-0309",
                "grok-4.20-0309-reasoning",
                "grok-4.20-0309-non-reasoning",
                "gpt-5.5",
                "gpt-5.3-codex-spark",
                "big-pickle",
                "deepseek-v4-flash-free",
                "minimax-m2.5-free",
                "nemotron-3-super-free",
                "kimi-k2.6",
                "qwen3.6-plus",
                "glm-5.1",
                "deepseek-v4-flash",
                "x-ai/grok-build-0.1",
                "poolside/laguna-m.1",
                "poolside/laguna-xs.2",
                "mimo-v2.5-pro",
                "mimo-v2-pro",
                "mimo-v2.5",
                "mimo-v2-omni",
                "mimo-v2-flash",
                "mimo-v2.5-pro",
                "mimo-v2-pro",
                "mimo-v2.5",
                "mimo-v2-omni",
                "mimo-v2-flash",
                "composer-2.5",
                "claude-opus-4-8",
                "claude-sonnet-4-6",
                "gpt-5.5",
                "gemini-3.1-pro-preview",
                "grok-4.3",
            ]
        );
    }

    #[test]
    fn provider_model_lists_match_gode_catalog() {
        assert_eq!(models_for_provider(PROVIDER_OPENAI, false).len(), 2);
        assert_eq!(models_for_codex(false).len(), 3);
        assert_eq!(models_for_provider(PROVIDER_ANTHROPIC, false).len(), 4);
        assert_eq!(models_for_provider(PROVIDER_CLAUDE_CODE, false).len(), 5);
        assert_eq!(models_for_provider(PROVIDER_GEMINI, false).len(), 5);
        assert_eq!(models_for_provider(PROVIDER_XAI, false).len(), 4);
        assert_eq!(models_for_provider(PROVIDER_SUPERGROK, false).len(), 4);
        assert_eq!(models_for_provider(PROVIDER_OPENCODE, false).len(), 6);
        assert_eq!(models_for_provider(PROVIDER_OPENCODE_GO, false).len(), 4);
        assert_eq!(models_for_provider(PROVIDER_OPENROUTER, false).len(), 1);
        assert_eq!(models_for_provider(PROVIDER_POOLSIDE, false).len(), 2);
        assert_eq!(models_for_provider(PROVIDER_CURSOR, false).len(), 6);
        assert_eq!(models_for_provider(PROVIDER_XIAOMI_MIMO, false).len(), 5);
        assert_eq!(
            models_for_provider(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN, false).len(),
            5
        );
        assert_eq!(models_for_provider(PROVIDER_MOCK, true).len(), 1);
    }

    #[test]
    fn claude_code_catalog_uses_long_context_windows() {
        let direct = lookup_model_for_provider(PROVIDER_ANTHROPIC, "claude-sonnet-4-6").unwrap();
        let claude_code =
            lookup_model_for_provider(PROVIDER_CLAUDE_CODE, "claude-sonnet-4-6").unwrap();

        assert_eq!(direct.context_window, 1_000_000);
        assert_eq!(claude_code.context_window, 1_000_000);
        assert_eq!(claude_code.auto_compact_token_limit, 900_000);
    }

    #[test]
    fn google_embedding_model_is_hidden_from_chat_lists() {
        assert!(lookup_model("gemini-embedding-2").is_some());
        assert!(
            models_for_provider(PROVIDER_GOOGLE, false)
                .iter()
                .all(|model| model.id != "gemini-embedding-2")
        );
        let model = lookup_model("gemini-embedding-2").unwrap();
        assert!(model.hidden);
        assert!(!model.supports_tools);
    }

    #[test]
    fn catalog_model_profile_derives_openai_defaults() {
        let profile = built_in_model_profile("gpt-5.5").unwrap();

        assert_eq!(profile.provider_family, ProviderFamily::OpenAi);
        assert_eq!(profile.edit_tool.as_deref(), Some(EDIT_TOOL_PATCH));
        assert_eq!(profile.schema_policy, ModelSchemaPolicy::RequiredFirstFlat);
        assert_eq!(
            profile.instruction_overlay,
            ModelInstructionOverlay::LiteralToolOutputs
        );
        assert_eq!(profile.reasoning.execution.as_deref(), Some(REASONING_LOW));
        assert_eq!(profile.parallel_tool_calls, Some(true));
    }

    #[test]
    fn poolside_catalog_defaults_to_thinking_enabled() {
        let laguna = lookup_model("poolside/laguna-m.1").unwrap();
        assert_eq!(laguna.default_reasoning, REASONING_MEDIUM);
        assert_eq!(
            laguna
                .supported_reasoning
                .iter()
                .map(|option| option.effort)
                .collect::<Vec<_>>(),
            vec![REASONING_NONE, REASONING_MEDIUM]
        );
    }

    #[test]
    fn xiaomi_mimo_catalog_uses_chat_completions_kind_and_exact_model_ids() {
        let provider = BUILT_IN_PROVIDERS
            .iter()
            .find(|provider| provider.id == PROVIDER_XIAOMI_MIMO)
            .unwrap();
        let token_plan = BUILT_IN_PROVIDERS
            .iter()
            .find(|provider| provider.id == PROVIDER_XIAOMI_MIMO_TOKEN_PLAN)
            .unwrap();

        assert_eq!(provider.kind, PROVIDER_KIND_CHAT_COMPLETIONS);
        assert_eq!(token_plan.kind, PROVIDER_KIND_CHAT_COMPLETIONS);
        assert_eq!(provider.env_key, Some("MIMO_API_KEY"));
        assert_eq!(token_plan.env_key, Some("MIMO_TOKEN_PLAN_API_KEY"));

        let ids = models_for_provider(PROVIDER_XIAOMI_MIMO, false)
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "mimo-v2.5-pro",
                "mimo-v2-pro",
                "mimo-v2.5",
                "mimo-v2-omni",
                "mimo-v2-flash"
            ]
        );
        assert!(lookup_model("out-of-v2-flash").is_none());
    }

    #[test]
    fn xai_catalog_entries_match_current_grok_contract() {
        let grok43 = models_for_provider(PROVIDER_XAI, false)
            .into_iter()
            .find(|model| model.id == "grok-4.3")
            .unwrap();
        assert_eq!(grok43.context_window, Some(1_000_000));
        assert_eq!(grok43.default_reasoning.as_deref(), Some(REASONING_LOW));
        assert_eq!(
            grok43
                .supported_reasoning
                .iter()
                .map(|option| option.effort.as_str())
                .collect::<Vec<_>>(),
            vec![
                REASONING_NONE,
                REASONING_LOW,
                REASONING_MEDIUM,
                REASONING_HIGH
            ]
        );

        let grok420 = lookup_model("grok-4.20-multi-agent-0309").unwrap();
        assert_eq!(grok420.context_window, 2_000_000);
        assert_eq!(grok420.auto_compact_token_limit, 1_800_000);
        assert_eq!(grok420.provider, PROVIDER_XAI);
    }

    #[test]
    fn provider_aliases_normalize_xai_and_supergrok() {
        assert_eq!(normalize_provider_id("grok"), PROVIDER_XAI);
        assert_eq!(normalize_provider_id("x.ai"), PROVIDER_XAI);
        assert_eq!(normalize_provider_id("x-ai"), PROVIDER_XAI);
        assert_eq!(normalize_provider_id("xai-oauth"), PROVIDER_SUPERGROK);
        assert_eq!(normalize_provider_id("grok-oauth"), PROVIDER_SUPERGROK);
        assert_eq!(normalize_provider_id("supergrok"), PROVIDER_SUPERGROK);
        assert_eq!(normalize_provider_id("laguna"), PROVIDER_POOLSIDE);
        assert_eq!(normalize_provider_id("composer"), PROVIDER_CURSOR);
    }

    #[test]
    fn cursor_catalog_profile_is_text_only_agentservice() {
        let composer = lookup_model("composer-2.5").unwrap();
        assert_eq!(composer.provider, PROVIDER_CURSOR);
        assert!(!composer.supports_tools);
        assert!(!composer.supports_structured);

        let profile = built_in_model_profile("composer-2.5").unwrap();
        assert_eq!(profile.provider_family, ProviderFamily::Cursor);
        assert_eq!(profile.parallel_tool_calls, Some(false));
    }

    #[test]
    fn provider_aware_lookup_resolves_cursor_proxied_models_to_cursor_family() {
        // Id-only lookup resolves shared ids to the first (native) entry.
        let id_only = built_in_model_profile("claude-opus-4-8").unwrap();
        assert_eq!(id_only.provider_family, ProviderFamily::Anthropic);

        // Provider-aware lookup resolves to the Cursor catalog entry/family.
        let cursor =
            built_in_model_profile_for_provider(PROVIDER_CURSOR, "claude-opus-4-8").unwrap();
        assert_eq!(cursor.provider_family, ProviderFamily::Cursor);
        assert_eq!(cursor.provider, PROVIDER_CURSOR);
        assert_eq!(cursor.parallel_tool_calls, Some(false));

        let anthropic =
            built_in_model_profile_for_provider(PROVIDER_ANTHROPIC, "claude-opus-4-8").unwrap();
        assert_eq!(anthropic.provider_family, ProviderFamily::Anthropic);

        // Unknown provider falls back to id-only resolution.
        let fallback =
            built_in_model_profile_for_provider("does-not-exist", "claude-opus-4-8").unwrap();
        assert_eq!(fallback.provider_family, ProviderFamily::Anthropic);
    }

    #[test]
    fn cursor_opus_advertises_configurable_reasoning_effort() {
        let opus = models_for_provider(PROVIDER_CURSOR, false)
            .into_iter()
            .find(|model| model.id == "claude-opus-4-8")
            .expect("cursor catalog should expose claude-opus-4-8");

        assert_eq!(opus.default_reasoning.as_deref(), Some(REASONING_HIGH));
        assert_eq!(
            opus.supported_reasoning
                .iter()
                .map(|option| option.effort.as_str())
                .collect::<Vec<_>>(),
            vec![
                REASONING_LOW,
                REASONING_MEDIUM,
                REASONING_HIGH,
                REASONING_XHIGH,
                REASONING_MAX
            ]
        );

        // Non-Opus Cursor models remain effort-free for now.
        let sonnet = models_for_provider(PROVIDER_CURSOR, false)
            .into_iter()
            .find(|model| model.id == "claude-sonnet-4-6")
            .expect("cursor catalog should expose claude-sonnet-4-6");
        assert_eq!(sonnet.default_reasoning, None);
        assert!(sonnet.supported_reasoning.is_empty());
    }

    #[test]
    fn claude_opus_and_sonnet_advertise_max_effort() {
        let efforts = |id: &str| {
            lookup_model(id)
                .unwrap()
                .supported_reasoning
                .iter()
                .map(|option| option.effort)
                .collect::<Vec<_>>()
        };

        // Opus 4.7/4.8 support both xhigh and max.
        for id in ["claude-opus-4-8", "claude-opus-4-7"] {
            assert_eq!(
                efforts(id),
                vec![
                    REASONING_LOW,
                    REASONING_MEDIUM,
                    REASONING_HIGH,
                    REASONING_XHIGH,
                    REASONING_MAX
                ],
                "{id} effort levels"
            );
        }

        // Sonnet 4.6 supports max but not xhigh.
        assert_eq!(
            efforts("claude-sonnet-4-6"),
            vec![
                REASONING_LOW,
                REASONING_MEDIUM,
                REASONING_HIGH,
                REASONING_MAX
            ]
        );

        // max stays Anthropic-specific; shared STANDARD_REASONING models do not gain it.
        assert!(!efforts("gpt-5.5").contains(&REASONING_MAX));
    }

    #[test]
    fn claude_haiku_does_not_advertise_reasoning_effort() {
        let haiku = lookup_model("claude-haiku-4-5-20251001").unwrap();

        assert_eq!(haiku.default_reasoning, REASONING_NONE);
        assert!(haiku.supported_reasoning.is_empty());

        let descriptor = ModelDescriptor::from(haiku);
        assert_eq!(descriptor.default_reasoning, None);
        assert!(descriptor.supported_reasoning.is_empty());
    }

    #[test]
    fn openai_context_windows_match_current_catalog_values() {
        let gpt55 = lookup_model("gpt-5.5").unwrap();
        assert_eq!(gpt55.context_window, 1_050_000);
        assert_eq!(gpt55.max_context_window, 1_050_000);
        assert_eq!(gpt55.auto_compact_token_limit, 945_000);

        let mini = lookup_model("gpt-5.4-mini").unwrap();
        assert_eq!(mini.context_window, 400_000);
        assert_eq!(mini.max_context_window, 400_000);
        assert_eq!(mini.auto_compact_token_limit, 360_000);

        let spark = lookup_model("gpt-5.3-codex-spark").unwrap();
        assert_eq!(spark.provider, PROVIDER_CODEX);
        assert_eq!(spark.context_window, 128_000);
        assert_eq!(spark.max_context_window, 128_000);
        assert_eq!(spark.auto_compact_token_limit, 115_200);
    }

    #[test]
    fn auto_compact_defaults_to_ninety_percent_of_context_window() {
        for model in BUILT_IN_MODELS {
            if model.context_window == 0 || model.auto_compact_token_limit == 0 {
                continue;
            }
            assert_eq!(
                model.auto_compact_token_limit,
                model.context_window.saturating_mul(9) / 10,
                "{} should compact at 90% of its context window",
                model.id
            );
        }
    }
}
