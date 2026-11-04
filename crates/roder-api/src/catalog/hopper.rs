use super::{
    ModelCatalogEntry, PROVIDER_HOPPER, PROVIDER_KIND_HOPPER, ProviderCatalogEntry, REASONING_HIGH,
    REASONING_LOW, REASONING_MAX, REASONING_NONE, REASONING_XHIGH, ReasoningOption,
};

/// Default OpenAI-compatible Chat Completions base URL for Hopper.
pub const HOPPER_DEFAULT_BASE_URL: &str = "https://api.withhopper.com/v1";

/// Default Hopper model id (newest DeepSeek V4.1 Flash snapshot).
pub const HOPPER_DEFAULT_MODEL: &str = "deepseek-v4.1-flash-1017";

/// Hopper-specific API-key env aliases. `HOPPER_API_KEY` is the primary key.
pub const HOPPER_ENV_ALIASES: &[&str] = &["RODER_HOPPER_API_KEY"];

pub(crate) const HOPPER_PROVIDER: ProviderCatalogEntry = ProviderCatalogEntry {
    id: PROVIDER_HOPPER,
    name: "Hopper",
    kind: PROVIDER_KIND_HOPPER,
    default_model: HOPPER_DEFAULT_MODEL,
    base_url: Some(HOPPER_DEFAULT_BASE_URL),
    env_key: Some("HOPPER_API_KEY"),
    env_aliases: HOPPER_ENV_ALIASES,
    requires_auth: true,
    supports_websockets: false,
};

/// Hopper serves DeepSeek with DeepSeek's thinking-mode wire format:
/// `thinking: { type: enabled|disabled }` plus `reasoning_effort`.
pub const HOPPER_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "Disable thinking mode",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low thinking effort",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High thinking effort",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extra-high thinking effort",
    },
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Maximum thinking effort",
    },
];

const HOPPER_CONTEXT_WINDOW: u32 = 1_048_576;

pub(crate) const HOPPER_DEEPSEEK_V4_1_FLASH_1017: ModelCatalogEntry = model(
    "deepseek-v4.1-flash-1017",
    "DeepSeek V4.1 Flash 1017",
    "DeepSeek V4.1 Flash (October 17, 2026 snapshot) on Hopper: 1M-token context, tool use and thinking.",
);

pub(crate) const HOPPER_DEEPSEEK_V4_1_FLASH_1017_ABLITERATED: ModelCatalogEntry = model(
    "deepseek-v4.1-flash-1017-abliterated",
    "DeepSeek V4.1 Flash 1017 Abliterated",
    "Abliterated (refusal-direction removed) DeepSeek V4.1 Flash 1017 on Hopper: 1M-token context, tool use and thinking.",
);

const fn model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_HOPPER,
        default_reasoning: REASONING_HIGH,
        supported_reasoning: HOPPER_REASONING,
        context_window: HOPPER_CONTEXT_WINDOW,
        max_context_window: HOPPER_CONTEXT_WINDOW,
        auto_compact_token_limit: HOPPER_CONTEXT_WINDOW.saturating_mul(9) / 10,
        supports_compaction: false,
        supports_images: true,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some("edit"),
        hidden: false,
    }
}
