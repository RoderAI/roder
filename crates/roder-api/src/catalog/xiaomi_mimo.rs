use super::{
    ModelCatalogEntry, PROVIDER_KIND_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO,
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN, ProviderCatalogEntry, REASONING_MEDIUM, STANDARD_REASONING,
};

pub const XIAOMI_MIMO_ENV_ALIASES: &[&str] = &["XIAOMI_MIMO_API_KEY", "RODER_XIAOMI_MIMO_API_KEY"];
pub const XIAOMI_MIMO_TOKEN_PLAN_ENV_ALIASES: &[&str] = &[
    "XIAOMI_MIMO_TOKEN_PLAN_API_KEY",
    "RODER_XIAOMI_MIMO_TOKEN_PLAN_API_KEY",
];

pub(crate) const PAY_AS_YOU_GO_PROVIDER: ProviderCatalogEntry = ProviderCatalogEntry {
    id: PROVIDER_XIAOMI_MIMO,
    name: "Xiaomi MiMo",
    kind: PROVIDER_KIND_XIAOMI_MIMO,
    default_model: "mimo-v2.5-pro",
    base_url: Some("https://api.xiaomimimo.com/v1"),
    env_key: Some("MIMO_API_KEY"),
    env_aliases: XIAOMI_MIMO_ENV_ALIASES,
    requires_auth: true,
    supports_websockets: false,
};

pub(crate) const TOKEN_PLAN_PROVIDER: ProviderCatalogEntry = ProviderCatalogEntry {
    id: PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    name: "Xiaomi MiMo Token Plan",
    kind: PROVIDER_KIND_XIAOMI_MIMO,
    default_model: "mimo-v2.5-pro",
    base_url: None,
    env_key: Some("MIMO_TOKEN_PLAN_API_KEY"),
    env_aliases: XIAOMI_MIMO_TOKEN_PLAN_ENV_ALIASES,
    requires_auth: true,
    supports_websockets: false,
};

pub(crate) const PAYG_V25_PRO: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO,
    "mimo-v2.5-pro",
    "MiMo V2.5 Pro",
    "Xiaomi MiMo flagship model for deep reasoning, long-context analysis, tools, and structured output.",
    1_000_000,
    128_000,
    false,
);

pub(crate) const PAYG_V2_PRO: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO,
    "mimo-v2-pro",
    "MiMo V2 Pro",
    "Xiaomi MiMo Pro-series text model for deep thinking and long-context work.",
    1_000_000,
    128_000,
    false,
);

pub(crate) const PAYG_V25: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO,
    "mimo-v2.5",
    "MiMo V2.5",
    "Xiaomi MiMo multimodal model for image, audio, and video understanding with tool support.",
    1_000_000,
    128_000,
    true,
);

pub(crate) const PAYG_V2_OMNI: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO,
    "mimo-v2-omni",
    "MiMo V2 Omni",
    "Xiaomi MiMo omni model for multimodal understanding.",
    256_000,
    128_000,
    true,
);

pub(crate) const PAYG_V2_FLASH: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO,
    "mimo-v2-flash",
    "MiMo V2 Flash",
    "Xiaomi MiMo flash model for high-concurrency, lower-cost text generation.",
    256_000,
    64_000,
    false,
);

pub(crate) const TOKEN_PLAN_V25_PRO: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    "mimo-v2.5-pro",
    "MiMo V2.5 Pro",
    "Token Plan Xiaomi MiMo flagship model for deep reasoning and long-context work.",
    1_000_000,
    128_000,
    false,
);

pub(crate) const TOKEN_PLAN_V2_PRO: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    "mimo-v2-pro",
    "MiMo V2 Pro",
    "Token Plan Xiaomi MiMo Pro-series text model.",
    1_000_000,
    128_000,
    false,
);

pub(crate) const TOKEN_PLAN_V25: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    "mimo-v2.5",
    "MiMo V2.5",
    "Token Plan Xiaomi MiMo multimodal model.",
    1_000_000,
    128_000,
    true,
);

pub(crate) const TOKEN_PLAN_V2_OMNI: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    "mimo-v2-omni",
    "MiMo V2 Omni",
    "Token Plan Xiaomi MiMo omni model.",
    256_000,
    128_000,
    true,
);

pub(crate) const TOKEN_PLAN_V2_FLASH: ModelCatalogEntry = model(
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    "mimo-v2-flash",
    "MiMo V2 Flash",
    "Token Plan Xiaomi MiMo flash model.",
    256_000,
    64_000,
    false,
);

const fn model(
    provider: &'static str,
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    max_output_tokens: u32,
    supports_images: bool,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider,
        default_reasoning: REASONING_MEDIUM,
        supported_reasoning: STANDARD_REASONING,
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
