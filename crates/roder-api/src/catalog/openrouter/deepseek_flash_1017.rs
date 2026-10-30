//! The October 17, 2026 DeepSeek Flash snapshot on OpenRouter.

use super::*;

pub(in crate::catalog) const DEEPSEEK_DEEPSEEK_V4_1_FLASH_1017: ModelCatalogEntry =
    openrouter_model(
        "deepseek/deepseek-v4.1-flash-1017",
        "DeepSeek V4.1 Flash 1017",
        "OpenRouter route for the October 17, 2026 DeepSeek V4.1 Flash snapshot: 1M-token context, image input, tool use and reasoning.",
        1_048_576,
        REASONING_HIGH,
        OPENROUTER_REASONING,
        true,
        true,
        EDIT_TOOL_EDIT,
    );

#[cfg(test)]
mod tests {
    use crate::catalog::{
        PROVIDER_OPENROUTER, lookup_model_for_provider, model_supports_reasoning_effort,
    };

    #[test]
    fn snapshot_exposes_browser_capabilities_and_client_compaction() {
        let model =
            lookup_model_for_provider(PROVIDER_OPENROUTER, "deepseek/deepseek-v4.1-flash-1017")
                .unwrap();
        assert!(model.supports_images);
        assert!(model.supports_tools);
        assert!(!model.supports_compaction);
        assert_eq!(model.auto_compact_token_limit, 943_718);
        assert!(model_supports_reasoning_effort(model.id, "high"));
        assert!(!model_supports_reasoning_effort(model.id, "max"));
    }
}
