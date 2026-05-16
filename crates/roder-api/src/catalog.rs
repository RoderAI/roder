use serde::Serialize;

use crate::inference::{ModelDescriptor, ReasoningEffortDescriptor};

pub const PROVIDER_MOCK: &str = "mock";
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_CODEX: &str = "codex";
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_GEMINI: &str = "gemini";

pub const PROVIDER_KIND_MOCK: &str = "mock";
pub const PROVIDER_KIND_OPENAI: &str = "openai";
pub const PROVIDER_KIND_CHAT_COMPLETIONS: &str = "chat_completions";
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_KIND_GEMINI: &str = "gemini";

pub const REASONING_NONE: &str = "none";
pub const REASONING_MINIMAL: &str = "minimal";
pub const REASONING_LOW: &str = "low";
pub const REASONING_MEDIUM: &str = "medium";
pub const REASONING_HIGH: &str = "high";
pub const REASONING_XHIGH: &str = "xhigh";

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
        effort: REASONING_NONE,
        description: "No explicit Gemini thinking configuration",
    },
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
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "High Gemini thinking with extra budget where supported",
    },
];

pub const MOCK_REASONING: &[ReasoningOption] = &[ReasoningOption {
    effort: REASONING_NONE,
    description: "No model-side reasoning",
}];

pub const GEMINI_ENV_ALIASES: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_API_KEY",
    "GOOGLE_AI_API_KEY",
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
        id: PROVIDER_GEMINI,
        name: "Gemini",
        kind: PROVIDER_KIND_GEMINI,
        default_model: "gemini-3.1-pro-preview",
        base_url: None,
        env_key: Some("GEMINI_API_TOKEN"),
        env_aliases: GEMINI_ENV_ALIASES,
        requires_auth: true,
        supports_websockets: false,
    },
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
        "claude-opus-4-7",
        "Claude Opus 4.7",
        "Most capable Claude model for complex reasoning and agentic coding.",
        1_000_000,
        900_000,
        REASONING_HIGH,
        STANDARD_REASONING,
    ),
    anthropic_model(
        "claude-sonnet-4-6",
        "Claude Sonnet 4.6",
        "Balanced Claude model for coding, tool use, and everyday agent workflows.",
        1_000_000,
        900_000,
        REASONING_MEDIUM,
        STANDARD_REASONING,
    ),
    anthropic_model(
        "claude-haiku-4-5-20251001",
        "Claude Haiku 4.5",
        "Fast Claude model for lower-latency tool workflows.",
        200_000,
        180_000,
        REASONING_LOW,
        HAIKU_REASONING,
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
        supports_tools: false,
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
        supports_tools: false,
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

impl From<&ModelCatalogEntry> for ModelDescriptor {
    fn from(model: &ModelCatalogEntry) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.display_name.to_string(),
            context_window: (model.context_window > 0).then_some(model.context_window),
            default_reasoning: Some(model.default_reasoning.to_string()),
            supported_reasoning: model
                .supported_reasoning
                .iter()
                .map(|option| ReasoningEffortDescriptor {
                    effort: option.effort.to_string(),
                    description: option.description.to_string(),
                })
                .collect(),
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
        assert_eq!(ids, vec!["mock", "openai", "codex", "anthropic", "gemini"]);
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
                "claude-opus-4-7",
                "claude-sonnet-4-6",
                "claude-haiku-4-5-20251001",
                "gemini-3.1-pro-preview",
                "gemini-3.1-pro-preview-customtools",
                "gemini-3-flash-preview",
                "gemini-3.1-flash-lite-preview",
            ]
        );
    }

    #[test]
    fn provider_model_lists_match_gode_catalog() {
        assert_eq!(models_for_provider(PROVIDER_OPENAI, false).len(), 2);
        assert_eq!(models_for_codex(false).len(), 3);
        assert_eq!(models_for_provider(PROVIDER_ANTHROPIC, false).len(), 3);
        assert_eq!(models_for_provider(PROVIDER_GEMINI, false).len(), 4);
        assert_eq!(models_for_provider(PROVIDER_MOCK, true).len(), 1);
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
