use super::{
    ModelCatalogEntry, PROVIDER_KIND_SYNTHETIC, PROVIDER_SYNTHETIC, ProviderCatalogEntry,
    REASONING_MEDIUM, STANDARD_REASONING,
};

/// Default Synthetic model alias. Synthetic documents that pinning concrete
/// `hf:` model ids risks 404s when older models rotate out, so the catalog
/// defaults to the always-up-to-date `syn:` alias.
pub const SYNTHETIC_DEFAULT_MODEL: &str = "syn:large:text";

/// Default OpenAI-compatible Chat Completions base URL. The endpoint-specific
/// OpenAI reference documents this URL; the general getting-started snippets
/// sometimes show `https://api.synthetic.new/v1`, which stays configurable via
/// `base_url`/`SYNTHETIC_BASE_URL`.
pub const SYNTHETIC_DEFAULT_BASE_URL: &str = "https://api.synthetic.new/openai/v1";

/// Synthetic-specific API-key env aliases. `SYNTHETIC_API_KEY` is the primary
/// key documented by Synthetic; `RODER_SYNTHETIC_API_KEY` is the Roder alias.
pub const SYNTHETIC_ENV_ALIASES: &[&str] = &["RODER_SYNTHETIC_API_KEY"];

pub(crate) const SYNTHETIC_PROVIDER: ProviderCatalogEntry = ProviderCatalogEntry {
    id: PROVIDER_SYNTHETIC,
    name: "Synthetic",
    kind: PROVIDER_KIND_SYNTHETIC,
    default_model: SYNTHETIC_DEFAULT_MODEL,
    base_url: Some(SYNTHETIC_DEFAULT_BASE_URL),
    env_key: Some("SYNTHETIC_API_KEY"),
    env_aliases: SYNTHETIC_ENV_ALIASES,
    requires_auth: true,
    supports_websockets: false,
};

pub(crate) const SYN_LARGE_TEXT: ModelCatalogEntry = model(
    "syn:large:text",
    "Synthetic Large (Text)",
    "Synthetic recommended large text alias routed to the latest flagship coding/reasoning model.",
    256_000,
    128_000,
    false,
);

pub(crate) const SYN_SMALL_TEXT: ModelCatalogEntry = model(
    "syn:small:text",
    "Synthetic Small (Text)",
    "Synthetic recommended small text alias routed to the latest fast, low-cost model.",
    256_000,
    128_000,
    false,
);

pub(crate) const SYN_LARGE_VISION: ModelCatalogEntry = model(
    "syn:large:vision",
    "Synthetic Large (Vision)",
    "Synthetic recommended large multimodal alias with image-input support.",
    256_000,
    128_000,
    true,
);

pub(crate) const SYN_SMALL_VISION: ModelCatalogEntry = model(
    "syn:small:vision",
    "Synthetic Small (Vision)",
    "Synthetic recommended small multimodal alias with image-input support.",
    256_000,
    128_000,
    true,
);

// --- Always-on concrete models -------------------------------------------
//
// Synthetic's "Always-On Models" are included in every subscription and
// exposed via the OpenAI-compatible `/models` endpoint. We pin them in the
// catalog so Roder can list them offline (no network discovery required) and
// can advertise their documented context windows. Synthetic still recommends
// the `syn:` aliases for routing, but the concrete ids are safe to select and
// stream. When a key is configured, Roder additionally discovers the live
// model list from `GET <base_url>/models` in the background and merges it.

pub(crate) const HF_MINIMAX_M3: ModelCatalogEntry = model(
    "hf:MiniMaxAI/MiniMax-M3",
    "MiniMax-M3",
    "Synthetic always-on Synthetic-hosted MiniMax-M3 reasoning model with 512k context.",
    524_288,
    128_000,
    false,
);

pub(crate) const HF_QWEN3_6_27B: ModelCatalogEntry = model(
    "hf:Qwen/Qwen3.6-27B",
    "Qwen3.6 27B",
    "Synthetic always-on Synthetic-hosted Qwen 3.6 27B model with 256k context.",
    262_144,
    128_000,
    false,
);

pub(crate) const HF_KIMI_K2_6: ModelCatalogEntry = model(
    "hf:moonshotai/Kimi-K2.6",
    "Kimi K2.6",
    "Synthetic always-on Synthetic-hosted Moonshot Kimi K2.6 model with 256k context.",
    262_144,
    128_000,
    false,
);

pub(crate) const HF_NEMOTRON_3_SUPER: ModelCatalogEntry = model(
    "hf:nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-NVFP4",
    "NVIDIA Nemotron-3 Super 120B",
    "Synthetic always-on Synthetic-hosted NVIDIA Nemotron-3 Super 120B (A12B NVFP4) with 256k context.",
    262_144,
    128_000,
    false,
);

pub(crate) const HF_GLM_4_7: ModelCatalogEntry = model(
    "hf:zai-org/GLM-4.7",
    "GLM-4.7",
    "Synthetic always-on Synthetic-hosted Zhipu GLM-4.7 model with 198k context.",
    202_752,
    128_000,
    false,
);

pub(crate) const HF_GLM_4_7_FLASH: ModelCatalogEntry = model(
    "hf:zai-org/GLM-4.7-Flash",
    "GLM-4.7 Flash",
    "Synthetic always-on Synthetic-hosted Zhipu GLM-4.7 Flash model with 192k context.",
    196_608,
    128_000,
    false,
);

pub(crate) const HF_GLM_5_1: ModelCatalogEntry = model(
    "hf:zai-org/GLM-5.1",
    "GLM-5.1",
    "Synthetic always-on Synthetic-hosted Zhipu GLM-5.1 model with 192k context.",
    196_608,
    128_000,
    false,
);

pub(crate) const HF_GLM_5_2: ModelCatalogEntry = model(
    "hf:zai-org/GLM-5.2",
    "GLM-5.2",
    "Synthetic always-on Synthetic-hosted Zhipu GLM-5.2 beta model with 512k context.",
    524_288,
    128_000,
    false,
);

pub(crate) const HF_GPT_OSS_120B: ModelCatalogEntry = model(
    "hf:openai/gpt-oss-120b",
    "GPT-OSS 120B",
    "Synthetic always-on Fireworks-hosted OpenAI gpt-oss-120b open-weight model with 128k context.",
    131_072,
    128_000,
    false,
);

pub(crate) const HF_QWEN3_5_397B_A17B: ModelCatalogEntry = model(
    "hf:Qwen/Qwen3.5-397B-A17B",
    "Qwen3.5 397B A17B",
    "Synthetic always-on Together AI-hosted Qwen 3.5 397B (A17B) model with 256k context.",
    262_144,
    128_000,
    false,
);

const fn model(
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
        provider: PROVIDER_SYNTHETIC,
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
