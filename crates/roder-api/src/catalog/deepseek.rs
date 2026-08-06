use super::{
    ModelCatalogEntry, PROVIDER_DEEPSEEK, PROVIDER_KIND_DEEPSEEK, ProviderCatalogEntry, REASONING_HIGH,
    REASONING_LOW, REASONING_MAX, REASONING_NONE, REASONING_XHIGH, ReasoningOption,
};

/// Default OpenAI-compatible Chat Completions base URL for DeepSeek Platform.
pub const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";

/// Default DeepSeek model id (non-thinking chat alias).
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";

/// DeepSeek-specific API-key env aliases. `DEEPSEEK_API_KEY` is the primary key.
pub const DEEPSEEK_ENV_ALIASES: &[&str] = &["RODER_DEEPSEEK_API_KEY"];

pub(crate) const DEEPSEEK_PROVIDER: ProviderCatalogEntry = ProviderCatalogEntry {
    id: PROVIDER_DEEPSEEK,
    name: "DeepSeek Platform",
    kind: PROVIDER_KIND_DEEPSEEK,
    default_model: DEEPSEEK_DEFAULT_MODEL,
    base_url: Some(DEEPSEEK_DEFAULT_BASE_URL),
    env_key: Some("DEEPSEEK_API_KEY"),
    env_aliases: DEEPSEEK_ENV_ALIASES,
    requires_auth: true,
    supports_websockets: false,
};

/// DeepSeek thinking-mode efforts from the Platform API docs.
///
/// Wire values are `low` / `high` / `max` (plus `xhigh`, which DeepSeek maps
/// per-model). `none` disables thinking via `thinking: { type: "disabled" }`.
/// Default effort is `high` when thinking is enabled.
pub const DEEPSEEK_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "Disable DeepSeek thinking mode",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Low DeepSeek thinking effort",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "High DeepSeek thinking effort (API default)",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extra-high DeepSeek thinking effort",
    },
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Maximum DeepSeek thinking effort",
    },
];

pub(crate) const DEEPSEEK_CHAT: ModelCatalogEntry = model(
    "deepseek-chat",
    "DeepSeek Chat",
    "DeepSeek non-thinking chat model alias (currently DeepSeek-V3.2 non-thinking mode).",
    128_000,
    8_192,
    false,
    REASONING_NONE,
    DEEPSEEK_REASONING,
);

pub(crate) const DEEPSEEK_REASONER: ModelCatalogEntry = model(
    "deepseek-reasoner",
    "DeepSeek Reasoner",
    "DeepSeek thinking/reasoner model alias (currently DeepSeek-V3.2 thinking mode).",
    128_000,
    64_000,
    false,
    REASONING_HIGH,
    DEEPSEEK_REASONING,
);

pub(crate) const DEEPSEEK_V4_FLASH: ModelCatalogEntry = model(
    "deepseek-v4-flash",
    "DeepSeek V4 Flash",
    "DeepSeek V4 Flash coding/chat model with optional thinking mode.",
    128_000,
    8_192,
    false,
    REASONING_HIGH,
    DEEPSEEK_REASONING,
);

pub(crate) const DEEPSEEK_V4_PRO: ModelCatalogEntry = model(
    "deepseek-v4-pro",
    "DeepSeek V4 Pro",
    "DeepSeek V4 Pro coding/reasoning model with thinking mode.",
    128_000,
    64_000,
    false,
    REASONING_HIGH,
    DEEPSEEK_REASONING,
);

const fn model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    max_output_tokens: u32,
    supports_images: bool,
    default_reasoning: &'static str,
    supported_reasoning: &'static [super::ReasoningOption],
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_DEEPSEEK,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit: context_window.saturating_mul(9) / 10,
        supports_compaction: false,
        supports_images,
        supports_tools: true,
        supports_structured: true,
        edit_tool: Some("edit"),
        hidden: max_output_tokens == 0,
    }
}
