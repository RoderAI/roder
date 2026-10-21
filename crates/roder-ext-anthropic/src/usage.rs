use roder_api::inference::TokenUsage;
use serde_json::Value;

/// `prompt_tokens` counts the full prompt: uncached input plus cache writes
/// plus cache reads, with the cache components also reported separately.
pub(crate) fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let uncached_prompt_tokens = number_to_u32(usage.get("input_tokens")).unwrap_or_default();
    let cache_creation_tokens =
        number_to_u32(usage.get("cache_creation_input_tokens")).unwrap_or_default();
    let cache_read_tokens = number_to_u32(usage.get("cache_read_input_tokens")).unwrap_or_default();
    let prompt_tokens = uncached_prompt_tokens
        .saturating_add(cache_creation_tokens)
        .saturating_add(cache_read_tokens);
    let completion_tokens = number_to_u32(usage.get("output_tokens")).unwrap_or_default();
    Some(
        TokenUsage::new(
            prompt_tokens,
            completion_tokens,
            prompt_tokens.saturating_add(completion_tokens),
        )
        .with_cached_prompt_tokens(cache_read_tokens)
        .with_cache_creation_prompt_tokens(cache_creation_tokens)
        .with_service_tier(
            usage
                .get("service_tier")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
    )
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_anthropic_usage_with_cache_components() {
        let value = json!({
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 1,
                "cache_read_input_tokens": 8,
                "output_tokens": 7
            }
        });
        assert_eq!(
            extract_usage(&value),
            Some(
                TokenUsage::new(11, 7, 18)
                    .with_cached_prompt_tokens(8)
                    .with_cache_creation_prompt_tokens(1)
            )
        );
    }

    #[test]
    fn retains_reported_tier_without_inventing_one_for_missing_or_malformed_values() {
        for tier in ["standard", "priority", "batch", "future-tier"] {
            let report =
                json!({"usage": {"input_tokens": 1, "output_tokens": 2, "service_tier": tier}});
            assert_eq!(
                extract_usage(&report).unwrap().service_tier.as_deref(),
                Some(tier)
            );
        }
        for report in [
            json!({"usage": {}}),
            json!({"usage": {"service_tier": null}}),
            json!({"usage": {"service_tier": 42}}),
        ] {
            assert_eq!(extract_usage(&report).unwrap().service_tier, None);
        }
        assert!(extract_usage(&json!({})).is_none());
    }
}
