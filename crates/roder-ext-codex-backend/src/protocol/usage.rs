use roder_api::backend::BackendEvent;
use roder_api::inference::TokenUsage;
use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
pub(super) struct UsageCounts {
    input: u64,
    output: u64,
    total: u64,
    cached: u64,
    cache_write: u64,
    reasoning: u64,
}

impl UsageCounts {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            input: value.get("inputTokens")?.as_u64()?,
            output: value.get("outputTokens")?.as_u64()?,
            total: value.get("totalTokens")?.as_u64()?,
            cached: value
                .get("cachedInputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_write: value
                .get("cacheWriteInputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            reasoning: value
                .get("reasoningOutputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })
    }

    fn subtract(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_sub(other.input),
            output: self.output.saturating_sub(other.output),
            total: self.total.saturating_sub(other.total),
            cached: self.cached.saturating_sub(other.cached),
            cache_write: self.cache_write.saturating_sub(other.cache_write),
            reasoning: self.reasoning.saturating_sub(other.reasoning),
        }
    }

    fn as_roder_usage(self) -> TokenUsage {
        let mut usage = TokenUsage::new(
            self.input.min(u32::MAX as u64) as u32,
            self.output.min(u32::MAX as u64) as u32,
            self.total.min(u32::MAX as u64) as u32,
        )
        .with_cached_prompt_tokens(self.cached.min(u32::MAX as u64) as u32);
        usage.cache_creation_prompt_tokens = self.cache_write.min(u32::MAX as u64) as u32;
        usage
    }
}

pub(super) fn usage_event(
    baseline: &mut Option<UsageCounts>,
    params: &Value,
) -> Option<(BackendEvent, BackendEvent)> {
    let usage = params.get("tokenUsage")?;
    let total = UsageCounts::parse(usage.get("total")?)?;
    let last = UsageCounts::parse(usage.get("last")?)?;
    let origin = baseline.get_or_insert_with(|| total.subtract(last));
    let current = total.subtract(*origin);
    Some((
        BackendEvent::Usage(current.as_roder_usage()),
        BackendEvent::ProviderMetadata(
            json!({"usage":{"output_tokens_details":{"reasoning_tokens":current.reasoning}}}),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accumulates_codex_usage_since_first_turn_update() {
        let mut baseline = None;
        let (first, _) = usage_event(&mut baseline, &json!({"tokenUsage":{
            "total":{"inputTokens":110,"outputTokens":30,"totalTokens":140,"cachedInputTokens":40,"reasoningOutputTokens":4},
            "last":{"inputTokens":10,"outputTokens":5,"totalTokens":15,"cachedInputTokens":2,"reasoningOutputTokens":1}
        }})).unwrap();
        let BackendEvent::Usage(first) = first else {
            panic!("usage expected")
        };
        assert_eq!(
            (
                first.prompt_tokens,
                first.completion_tokens,
                first.total_tokens
            ),
            (10, 5, 15)
        );
        let (next, metadata) = usage_event(&mut baseline, &json!({"tokenUsage":{
            "total":{"inputTokens":130,"outputTokens":40,"totalTokens":170,"cachedInputTokens":45,"reasoningOutputTokens":7},
            "last":{"inputTokens":20,"outputTokens":10,"totalTokens":30,"cachedInputTokens":5,"reasoningOutputTokens":3}
        }})).unwrap();
        let BackendEvent::Usage(next) = next else {
            panic!("usage expected")
        };
        assert_eq!(
            (
                next.prompt_tokens,
                next.completion_tokens,
                next.total_tokens,
                next.cached_prompt_tokens
            ),
            (30, 15, 45, 7)
        );
        let BackendEvent::ProviderMetadata(metadata) = metadata else {
            panic!("reasoning metadata expected")
        };
        assert_eq!(
            metadata["usage"]["output_tokens_details"]["reasoning_tokens"],
            4
        );
    }
}
