pub trait ClipboardSink {
    fn write_text(&mut self, text: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct MemoryClipboardSink {
    pub writes: Vec<String>,
}

impl ClipboardSink for MemoryClipboardSink {
    fn write_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.writes.push(text.to_string());
        Ok(())
    }
}

pub fn copy_selection(sink: &mut dyn ClipboardSink, text: &str) -> anyhow::Result<bool> {
    if text.is_empty() {
        return Ok(false);
    }
    sink.write_text(text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_selection_writes_to_injected_clipboard() {
        let mut sink = MemoryClipboardSink::default();

        assert!(copy_selection(&mut sink, "selected").unwrap());
        assert_eq!(sink.writes, ["selected"]);
    }
}
