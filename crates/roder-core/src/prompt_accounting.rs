use roder_api::transcript::TranscriptItem;
use serde_json::Value;
use std::collections::HashSet;

/// Estimates opaque provider state and image input separately from canonical
/// text. These are conservative estimates, never billing or payload-byte claims.
pub(crate) fn extra_prompt_tokens(items: &[TranscriptItem]) -> u32 {
    let mut seen = HashSet::new();
    let mut tokens = 0u32;
    for item in items {
        match item {
            TranscriptItem::UserMessage(message) => {
                tokens = tokens.saturating_add((message.images.len() as u32).saturating_mul(1_600));
            }
            TranscriptItem::ToolResult(result)
                if result
                    .display_payload
                    .as_ref()
                    .and_then(|payload| payload.get(roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY))
                    .is_some() =>
            {
                tokens = tokens.saturating_add(1_600);
            }
            TranscriptItem::ProviderMetadata(metadata) => {
                if let Some(window) = metadata.get("compacted_input") {
                    tokens = tokens.saturating_add(value_tokens(window));
                    continue;
                }
                for raw in metadata
                    .get("output")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let key = raw
                        .get("id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| raw.to_string());
                    if !seen.insert(key) {
                        continue;
                    }
                    if matches!(
                        raw["type"].as_str(),
                        Some("reasoning" | "compaction" | "compaction_summary")
                    ) && let Some(opaque) = raw.get("encrypted_content").and_then(Value::as_str)
                    {
                        tokens = tokens.saturating_add(
                            opaque.len().div_ceil(4).try_into().unwrap_or(u32::MAX),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    tokens
}

pub(crate) fn value_tokens(value: &Value) -> u32 {
    match value {
        Value::String(text) => text.len().div_ceil(4).try_into().unwrap_or(u32::MAX),
        Value::Array(items) => items
            .iter()
            .fold(0u32, |sum, item| sum.saturating_add(value_tokens(item))),
        Value::Object(fields) => fields.iter().fold(0u32, |sum, (key, value)| {
            sum.saturating_add(if key == "image_url" {
                1_600
            } else {
                value_tokens(value)
            })
        }),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn counts_encrypted_state_once_and_excludes_transport_telemetry() {
        let raw = json!({"id":"reason1","type":"reasoning","encrypted_content":"x".repeat(4_000)});
        assert_eq!(
            extra_prompt_tokens(&[
                TranscriptItem::ProviderMetadata(json!({"output":[raw]})),
                TranscriptItem::ProviderMetadata(json!({"output":[raw]})),
                TranscriptItem::ProviderMetadata(json!({"diagnostics":"x".repeat(8_000)}))
            ]),
            1_000
        );
    }
    #[test]
    fn images_count_as_image_estimates_not_base64_text_tokens() {
        assert_eq!(
            value_tokens(
                &json!({"type":"input_image","image_url":"data:image/png;base64,".to_string()+&"A".repeat(80_000)})
            ),
            1_603
        );
    }
}
