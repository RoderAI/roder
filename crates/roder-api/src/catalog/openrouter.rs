//! OpenRouter model catalog.
//!
//! OpenRouter lists several hundred routes and `list_models` discovers all of
//! them at runtime, but discovery only carries a context length. Entries here
//! add what the runtime cannot learn from discovery: the reasoning ladder, the
//! compaction threshold, image support, and the edit tool. Values come from the
//! live `https://openrouter.ai/api/v1/models` listing (refreshed 2026-10-09); the
//! ids are OpenRouter slugs, so they never collide with first-party ids.
//!
//! `supports_compaction` is deliberately `false`: OpenRouter accepts the
//! OpenAI `context_management` field but does not compact server-side, so
//! roder must compact client-side at `auto_compact_token_limit` (90% of the
//! window) instead of waiting for a server that never does it.

use super::{
    EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, ModelCatalogEntry, OPENROUTER_REASONING, PROVIDER_OPENROUTER,
    REASONING_HIGH, REASONING_MEDIUM, REASONING_NONE, ReasoningOption,
};

#[allow(clippy::too_many_arguments)]
const fn openrouter_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    context_window: u32,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
    supports_images: bool,
    supports_structured: bool,
    edit_tool: &'static str,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_OPENROUTER,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window: context_window,
        auto_compact_token_limit: context_window.saturating_mul(9) / 10,
        supports_compaction: false,
        supports_images,
        supports_tools: true,
        supports_structured,
        edit_tool: Some(edit_tool),
        hidden: false,
    }
}

pub(super) const X_AI_GROK_4_6: ModelCatalogEntry = openrouter_model(
    "x-ai/grok-4.6",
    "Grok 4.6",
    "OpenRouter route for xAI's flagship model for coding and long-running agent workflows.",
    500_000,
    REASONING_HIGH,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const X_AI_GROK_4_7: ModelCatalogEntry = openrouter_model(
    "x-ai/grok-4.7",
    "Grok 4.7",
    "OpenRouter route for Grok 4.7 (x-ai): 500K-token context, up to 450K output tokens, tool use, image input, reasoning.",
    500_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const MOONSHOTAI_KIMI_K3: ModelCatalogEntry = openrouter_model(
    "moonshotai/kimi-k3",
    "Kimi K3",
    "OpenRouter route for Kimi K3 (moonshotai): 1M-token context, up to 943K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const MOONSHOTAI_KIMI_K2_7_CODE: ModelCatalogEntry = openrouter_model(
    "moonshotai/kimi-k2.7-code",
    "Kimi K2.7 Code",
    "OpenRouter route for Kimi K2.7 Code (moonshotai): 262K-token context, up to 235K output tokens, tool use, image input, reasoning.",
    262_144,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const MOONSHOTAI_KIMI_K2_6: ModelCatalogEntry = openrouter_model(
    "moonshotai/kimi-k2.6",
    "Kimi K2.6",
    "OpenRouter route for Kimi K2.6 (moonshotai): 262K-token context, up to 235K output tokens, tool use, image input, reasoning.",
    262_144,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const ANTHROPIC_CLAUDE_OPUS_5_5: ModelCatalogEntry = openrouter_model(
    "anthropic/claude-opus-5.5",
    "Claude Opus 5.5",
    "OpenRouter route for Claude Opus 5.5 (anthropic): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const ANTHROPIC_CLAUDE_SONNET_5_5: ModelCatalogEntry = openrouter_model(
    "anthropic/claude-sonnet-5.5",
    "Claude Sonnet 5.5",
    "OpenRouter route for Claude Sonnet 5.5 (anthropic): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const ANTHROPIC_CLAUDE_HAIKU_5_5: ModelCatalogEntry = openrouter_model(
    "anthropic/claude-haiku-5.5",
    "Claude Haiku 5.5",
    "OpenRouter route for Claude Haiku 5.5 (anthropic): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const ANTHROPIC_CLAUDE_FABLE_5_1: ModelCatalogEntry = openrouter_model(
    "anthropic/claude-fable-5.1",
    "Claude Fable 5.1",
    "OpenRouter route for Claude Fable 5.1 (anthropic): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const ANTHROPIC_CLAUDE_OPUS_5: ModelCatalogEntry = openrouter_model(
    "anthropic/claude-opus-5",
    "Claude Opus 5",
    "OpenRouter route for Claude Opus 5 (anthropic): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const OPENAI_GPT_6_1_SOL: ModelCatalogEntry = openrouter_model(
    "openai/gpt-6.1-sol",
    "GPT-6.1 Sol",
    "OpenRouter route for GPT-6.1 Sol (openai): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_050_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const OPENAI_GPT_6_SOL: ModelCatalogEntry = openrouter_model(
    "openai/gpt-6-sol",
    "GPT-6 Sol",
    "OpenRouter route for GPT-6 Sol (openai): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_050_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const OPENAI_GPT_6_LUNA: ModelCatalogEntry = openrouter_model(
    "openai/gpt-6-luna",
    "GPT-6 Luna",
    "OpenRouter route for GPT-6 Luna (openai): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_050_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const OPENAI_GPT_6_ASTRA: ModelCatalogEntry = openrouter_model(
    "openai/gpt-6-astra",
    "GPT-6 Astra",
    "OpenRouter route for GPT-6 Astra (openai): 1M-token context, up to 128K output tokens, tool use, image input, reasoning.",
    1_050_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_PATCH,
);

pub(super) const GOOGLE_GEMINI_3_8_FLASH: ModelCatalogEntry = openrouter_model(
    "google/gemini-3.8-flash",
    "Gemini 3.8 Flash",
    "OpenRouter route for Gemini 3.8 Flash (google): 1M-token context, up to 65K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const GOOGLE_GEMINI_3_7_FLASH: ModelCatalogEntry = openrouter_model(
    "google/gemini-3.7-flash",
    "Gemini 3.7 Flash",
    "OpenRouter route for Gemini 3.7 Flash (google): 1M-token context, up to 65K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const DEEPSEEK_DEEPSEEK_V4_PRO_0813: ModelCatalogEntry = openrouter_model(
    "deepseek/deepseek-v4-pro-0813",
    "DeepSeek V4 Pro 0813",
    "OpenRouter route for DeepSeek V4 Pro 0813 (deepseek): 1M-token context, up to 393K output tokens, tool use, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    false,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const DEEPSEEK_DEEPSEEK_V4_1_FLASH: ModelCatalogEntry = openrouter_model(
    "deepseek/deepseek-v4.1-flash",
    "DeepSeek V4.1 Flash",
    "OpenRouter route for DeepSeek V4.1 Flash (deepseek): 1M-token context, up to 943K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const QWEN_QWEN3_8_MAX_0902: ModelCatalogEntry = openrouter_model(
    "qwen/qwen3.8-max-0902",
    "Qwen3.8 Max (0902)",
    "OpenRouter route for Qwen3.8 Max (0902) (qwen): 1M-token context, up to 131K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const QWEN_QWEN3_8_FLASH: ModelCatalogEntry = openrouter_model(
    "qwen/qwen3.8-flash",
    "Qwen3.8 Flash",
    "OpenRouter route for Qwen3.8 Flash (qwen): 1M-token context, up to 131K output tokens, tool use, image input, reasoning.",
    1_000_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const Z_AI_GLM_5_3: ModelCatalogEntry = openrouter_model(
    "z-ai/glm-5.3",
    "GLM 5.3",
    "OpenRouter route for GLM 5.3 (z-ai): 1M-token context, up to 943K output tokens, tool use, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    false,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const Z_AI_GLM_5_3_FLASH: ModelCatalogEntry = openrouter_model(
    "z-ai/glm-5.3-flash",
    "GLM 5.3 Flash",
    "OpenRouter route for GLM 5.3 Flash (z-ai): 1M-token context, up to 943K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const XIAOMI_MIMO_V2_6_PRO: ModelCatalogEntry = openrouter_model(
    "xiaomi/mimo-v2.6-pro",
    "MiMo-V2.6-Pro",
    "OpenRouter route for MiMo-V2.6-Pro (xiaomi): 1M-token context, up to 131K output tokens, tool use, image input, reasoning.",
    1_050_000,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const MISTRALAI_MISTRAL_LARGE_4_0: ModelCatalogEntry = openrouter_model(
    "mistralai/mistral-large-4-0",
    "Mistral Large 4",
    "OpenRouter route for Mistral Large 4 (mistralai): 1M-token context, up to 262K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);

pub(super) const META_MUSE_SPARK_1_3: ModelCatalogEntry = openrouter_model(
    "meta/muse-spark-1.3",
    "Muse Spark 1.3",
    "OpenRouter route for Muse Spark 1.3 (meta): 1M-token context, up to 943K output tokens, tool use, image input, reasoning.",
    1_048_576,
    REASONING_MEDIUM,
    OPENROUTER_REASONING,
    true,
    true,
    EDIT_TOOL_EDIT,
);
