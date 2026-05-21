use roder_api::memory::MemorySearchResult;
use serde::Deserialize;

pub(crate) const CONCISE_MEMORY_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseFormat {
    Concise,
    Detailed,
}

impl Default for ResponseFormat {
    fn default() -> Self {
        Self::Concise
    }
}

impl ResponseFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }

    pub(crate) fn format_memory_text(self, text: &str) -> String {
        match self {
            Self::Concise => bounded(text, CONCISE_MEMORY_CHARS),
            Self::Detailed => text.to_string(),
        }
    }
}

pub(crate) fn render_memory_query_results(
    results: &[MemorySearchResult],
    response_format: ResponseFormat,
) -> String {
    if results.is_empty() {
        return "0 memories".to_string();
    }

    let mut text = format!("{} memories:", results.len());
    for (index, result) in results.iter().enumerate() {
        let id = result.record.id.as_deref().unwrap_or("<unsaved>");
        text.push_str(&format!("\n{}. {id}", index + 1));
        text.push_str(&format!(
            "\n   {}",
            response_format.format_memory_text(&result.record.text)
        ));
    }
    text
}

fn bounded(input: &str, max_chars: usize) -> String {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let suffix = "...";
    let keep = max_chars.saturating_sub(suffix.len());
    let mut output = normalized.chars().take(keep).collect::<String>();
    output.push_str(suffix);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::memory::{MemoryRecord, MemoryScope, MemorySearchResult};
    use time::OffsetDateTime;

    #[test]
    fn response_format_concise_bounds_memory_text() {
        let full = "memory ".repeat(80);
        let concise = ResponseFormat::Concise.format_memory_text(&full);
        let detailed = ResponseFormat::Detailed.format_memory_text(&full);

        assert!(concise.len() < detailed.len());
        assert!(concise.contains("..."));
        assert_eq!(detailed, full);
    }

    #[test]
    fn response_format_rendered_query_includes_ids() {
        let now = OffsetDateTime::now_utc();
        let result = MemorySearchResult {
            record: MemoryRecord {
                id: Some("memory-1".to_string()),
                scope: MemoryScope::Project("p".to_string()),
                text: "memory ".repeat(80),
                content_hash: None,
                metadata: serde_json::json!({}),
                usage: None,
                deleted: false,
                created_at: now,
                updated_at: now,
            },
            score: 1.0,
            citation: None,
        };

        let text = render_memory_query_results(&[result], ResponseFormat::Concise);

        assert!(text.contains("memory-1"));
        assert!(text.contains("..."));
    }
}
