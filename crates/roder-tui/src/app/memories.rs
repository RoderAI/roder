use roder_api::memory::{MemoryCitation, MemoryRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTimelineRow {
    pub title: String,
    pub detail: String,
}

pub fn memory_row(memory: &MemoryRecord) -> MemoryTimelineRow {
    MemoryTimelineRow {
        title: format!(
            "{} {}",
            memory.scope.stable_id(),
            memory.id.clone().unwrap_or_default()
        )
        .trim()
        .to_string(),
        detail: compact(&memory.text, 96),
    }
}

pub fn citation_label(citation: &MemoryCitation) -> String {
    format!(
        "{} {:.2}",
        citation.scope_id,
        citation.score_millis as f32 / 1000.0
    )
}

fn compact(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        text
    } else {
        let mut out = text.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::memory::MemoryScope;
    use time::OffsetDateTime;

    #[test]
    fn memories_row_includes_scope_and_compact_detail() {
        let row = memory_row(&MemoryRecord {
            id: Some("m1".to_string()),
            scope: MemoryScope::Project("gode".to_string()),
            text: "remember the sqlite vector memory provider".to_string(),
            content_hash: None,
            metadata: serde_json::json!({}),
            usage: None,
            deleted: false,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        });
        assert!(row.title.contains("project:gode"));
        assert!(row.detail.contains("sqlite vector"));
    }
}
