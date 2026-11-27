use super::{PROVIDER_CODEX, PROVIDER_OPENAI, lookup_model_for_provider};

/// Resolve the working context budget, clamping native GPT overrides to the
/// model's supported maximum without changing other providers' catalog limits.
pub fn effective_model_context_window(
    provider: &str,
    model: &str,
    configured: Option<u32>,
) -> Option<u32> {
    let entry = lookup_model_for_provider(provider, model)?;
    let window =
        if matches!(provider, PROVIDER_OPENAI | PROVIDER_CODEX) && model.starts_with("gpt-") {
            configured
                .filter(|window| *window > 0)
                .map(|window| window.min(entry.max_context_window))
                .unwrap_or(entry.context_window)
        } else {
            entry.context_window
        };
    (window > 0).then_some(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{PROVIDER_ANTHROPIC, PROVIDER_CURSOR};

    #[test]
    fn native_gpt_defaults_and_overrides_respect_supported_maxima() {
        for provider in [PROVIDER_OPENAI, PROVIDER_CODEX] {
            for model in [
                "gpt-6.2-sol",
                "gpt-6.2-astra",
                "gpt-6.1-sol",
                "gpt-6.1-terra",
                "gpt-6-astra",
                "gpt-6-luna",
            ] {
                assert_eq!(
                    effective_model_context_window(provider, model, None),
                    Some(500_000)
                );
                assert_eq!(
                    effective_model_context_window(provider, model, Some(300_000)),
                    Some(300_000)
                );
                assert_eq!(
                    effective_model_context_window(provider, model, Some(1_000_000)),
                    Some(872_000)
                );
            }
            assert_eq!(
                effective_model_context_window(provider, "gpt-5.5", Some(500_000)),
                Some(272_000)
            );
            assert_eq!(
                effective_model_context_window(provider, "gpt-6.2-sol", Some(0)),
                Some(500_000)
            );
        }
    }

    #[test]
    fn overrides_do_not_change_other_providers_or_unknown_models() {
        for (provider, model) in [
            (PROVIDER_ANTHROPIC, "claude-sonnet-4-6"),
            (PROVIDER_CURSOR, "gpt-5.5"),
        ] {
            assert_eq!(
                effective_model_context_window(provider, model, Some(250_000)),
                effective_model_context_window(provider, model, None)
            );
        }
        assert_eq!(
            effective_model_context_window(PROVIDER_OPENAI, "gpt-unknown", Some(500_000)),
            None
        );
    }
}
