use roder_api::context::{ContextBlock, ContextBlockKind};

pub fn saved_context_block(id: impl Into<String>, text: impl Into<String>) -> ContextBlock {
    ContextBlock {
        id: id.into(),
        kind: ContextBlockKind::PriorSummary,
        text: text.into(),
        priority: 50,
        token_estimate: None,
        metadata: serde_json::json!({ "source": "disk-context" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_saved_context_block() {
        let block = saved_context_block("ctx", "summary");
        assert_eq!(block.id, "ctx");
        assert!(matches!(block.kind, ContextBlockKind::PriorSummary));
    }
}
