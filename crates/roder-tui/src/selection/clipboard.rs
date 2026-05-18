use async_trait::async_trait;

#[async_trait]
pub trait ClipboardSink: Send + Sync {
    async fn write_text(&self, text: &str) -> anyhow::Result<()>;
}

pub async fn copy_selection(
    sink: &dyn ClipboardSink,
    text: impl AsRef<str>,
) -> anyhow::Result<bool> {
    let text = text.as_ref();
    if text.trim().is_empty() {
        return Ok(false);
    }
    sink.write_text(text).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct FakeClipboard {
        writes: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ClipboardSink for FakeClipboard {
        async fn write_text(&self, text: &str) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn copy_selection_writes_through_injected_clipboard() {
        let clipboard = FakeClipboard::default();

        assert!(copy_selection(&clipboard, "selected text").await.unwrap());
        assert_eq!(
            clipboard.writes.lock().unwrap().as_slice(),
            &["selected text".to_string()]
        );
    }

    #[tokio::test]
    async fn copy_selection_ignores_empty_text() {
        let clipboard = FakeClipboard::default();

        assert!(!copy_selection(&clipboard, "  \n").await.unwrap());
        assert!(clipboard.writes.lock().unwrap().is_empty());
    }
}
