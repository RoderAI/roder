use super::{
    EDIT_TOOL_PATCH, ModelCatalogEntry, PROVIDER_OPENAI, REASONING_HIGH, REASONING_LOW,
    REASONING_MAX, REASONING_MEDIUM, REASONING_NONE, REASONING_ULTRA, REASONING_XHIGH,
    ReasoningOption, STANDARD_REASONING,
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

const GPT_6_REASONING: &[ReasoningOption] = &[
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

const GPT_6_SOL_LUNA_REASONING: &[ReasoningOption] = &[
    ReasoningOption {
        effort: REASONING_NONE,
        description: "No reasoning for fastest responses",
    },
    ReasoningOption {
        effort: REASONING_LOW,
        description: "Fast responses with lighter reasoning",
    },
    ReasoningOption {
        effort: REASONING_MEDIUM,
        description: "Balanced reasoning",
    },
    ReasoningOption {
        effort: REASONING_HIGH,
        description: "Deeper reasoning",
    },
    ReasoningOption {
        effort: REASONING_XHIGH,
        description: "Extra high reasoning",
    },
    ReasoningOption {
        effort: REASONING_MAX,
        description: "Maximum reasoning",
    },
];

pub(super) const GPT_62_ASTRA: ModelCatalogEntry = openai_codex_model(
    "gpt-6.2-astra",
    "GPT-6.2 Astra",
    "OpenAI's most capable model for complex reasoning and coding.",
    REASONING_LOW,
    GPT_56_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_62_SOL: ModelCatalogEntry = openai_codex_model(
    "gpt-6.2-sol",
    "GPT-6.2 Sol",
    "Everyday coding and agentic work balancing intelligence and cost.",
    REASONING_LOW,
    GPT_56_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_6_ASTRA: ModelCatalogEntry = openai_codex_model(
    "gpt-6-astra",
    "GPT-6-Astra",
    "OpenAI's most capable model, built for the hardest end-to-end work.",
    REASONING_HIGH,
    GPT_6_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_6_SOL: ModelCatalogEntry = openai_codex_model(
    "gpt-6-sol",
    "GPT-6 Sol",
    "Agentic coding model balancing intelligence and cost.",
    REASONING_MEDIUM,
    GPT_6_SOL_LUNA_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_61_SOL: ModelCatalogEntry = openai_codex_model(
    "gpt-6.1-sol",
    "GPT-6.1-Sol",
    "Agentic coding model balancing intelligence and cost.",
    REASONING_MEDIUM,
    GPT_6_SOL_LUNA_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_61_TERRA: ModelCatalogEntry = openai_codex_model(
    "gpt-6.1-terra",
    "GPT-6.1 Terra",
    "Fast computer-use and coding model with a 500k default context window. $1 / $5 per 1M input/output tokens.",
    REASONING_MEDIUM,
    GPT_6_SOL_LUNA_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_6_LUNA: ModelCatalogEntry = openai_codex_model(
    "gpt-6-luna",
    "GPT-6 Luna",
    "Efficient model for focused, high-volume tasks.",
    REASONING_MEDIUM,
    GPT_6_SOL_LUNA_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_56_SOL: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-sol",
    "GPT-5.6-Sol",
    "Latest frontier agentic coding model.",
    REASONING_LOW,
    GPT_56_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_56_TERRA: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-terra",
    "GPT-5.6-Terra",
    "Balanced agentic coding model for everyday work.",
    REASONING_MEDIUM,
    GPT_56_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_56_LUNA: ModelCatalogEntry = openai_codex_model(
    "gpt-5.6-luna",
    "GPT-5.6-Luna",
    "Fast and affordable agentic coding model.",
    REASONING_MEDIUM,
    GPT_56_LUNA_REASONING,
    500_000,
    872_000,
);

pub(super) const GPT_54: ModelCatalogEntry = openai_codex_model(
    "gpt-5.4",
    "GPT-5.4",
    "Strong model for everyday coding.",
    REASONING_MEDIUM,
    STANDARD_REASONING,
    500_000,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PROVIDER_CODEX, lookup_model_for_provider, models_for_codex, models_for_provider};

    #[test]
    fn gpt_62_models_are_selectable_with_reasoning_and_context_metadata() {
        for (id, name, default_reasoning) in [
            ("gpt-6.2-astra", "GPT-6.2 Astra", REASONING_LOW),
            ("gpt-6.2-sol", "GPT-6.2 Sol", REASONING_LOW),
        ] {
            for provider in [PROVIDER_OPENAI, PROVIDER_CODEX] {
                let model = lookup_model_for_provider(provider, id).unwrap();
                assert_eq!(model.display_name, name);
                assert_eq!(model.default_reasoning, default_reasoning);
                assert_eq!(model.context_window, 500_000);
                assert_eq!(model.max_context_window, 872_000);
                assert_eq!(model.auto_compact_token_limit, 450_000);
                assert_eq!(
                    model
                        .supported_reasoning
                        .iter()
                        .map(|option| option.effort)
                        .collect::<Vec<_>>(),
                    [
                        REASONING_LOW,
                        REASONING_MEDIUM,
                        REASONING_HIGH,
                        REASONING_XHIGH,
                        REASONING_MAX,
                        REASONING_ULTRA,
                    ]
                );
                assert!(model.supports_tools);
                assert!(model.supports_images);
                assert!(model.supports_compaction);
                assert!(!model.supports_structured);
                assert!(!model.hidden);
            }
            assert!(
                models_for_provider(PROVIDER_OPENAI, false)
                    .iter()
                    .any(|model| model.id == id)
            );
            assert!(models_for_codex(false).iter().any(|model| model.id == id));
        }
    }
}
