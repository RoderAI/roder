use super::{
    EDIT_TOOL_PATCH, ModelCatalogEntry, PROVIDER_OPENAI, REASONING_HIGH, REASONING_LOW,
    REASONING_MAX, REASONING_MEDIUM, REASONING_ULTRA, REASONING_XHIGH, ReasoningOption,
    STANDARD_REASONING,
};

const GPT_56_REASONING: &[ReasoningOption] = &[
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
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Maximum reasoning depth for the hardest problems",
    },
    ReasoningOption {
        effort: REASONING_ULTRA,
        description: "Maximum reasoning with automatic task delegation",
    },
];

const GPT_56_LUNA_REASONING: &[ReasoningOption] = &[
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
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Maximum reasoning depth for the hardest problems",
    },
];

pub(super) const GPT_56_SOL: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-sol",
    "GPT-5.6-Sol",
    "Latest frontier agentic coding model.",
    REASONING_LOW,
    GPT_56_REASONING,
    372_000,
    372_000,
);

pub(super) const GPT_56_TERRA: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-terra",
    "GPT-5.6-Terra",
    "Balanced agentic coding model for everyday work.",
    REASONING_MEDIUM,
    GPT_56_REASONING,
    372_000,
    372_000,
);

pub(super) const GPT_56_LUNA: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-luna",
    "GPT-5.6-Luna",
    "Fast and affordable agentic coding model.",
    REASONING_MEDIUM,
    GPT_56_LUNA_REASONING,
    372_000,
    372_000,
);

pub(super) const GPT_54: ModelCatalogEntry = openai_codex_model(
    "gpt-5.4",
    "GPT-5.4",
    "Strong model for everyday coding.",
    REASONING_MEDIUM,
    STANDARD_REASONING,
    272_000,
    1_000_000,
);

const fn openai_codex_model(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    default_reasoning: &'static str,
    supported_reasoning: &'static [ReasoningOption],
    context_window: u32,
    max_context_window: u32,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id,
        display_name,
        description,
        provider: PROVIDER_OPENAI,
        default_reasoning,
        supported_reasoning,
        context_window,
        max_context_window,
        auto_compact_token_limit: context_window.saturating_mul(9) / 10,
        supports_compaction: true,
        supports_images: true,
        supports_tools: true,
        supports_structured: false,
        edit_tool: Some(EDIT_TOOL_PATCH),
        hidden: false,
    }
}
