use crate::mouse::SelectedText;
use crate::selection::{ClipboardSink, copy_selection};

#[derive(Debug, Clone, Default)]
pub(super) struct SelectionKeyboardState {
    last_selection: Option<SelectedText>,
    clipboard: TextClipboardSink,
}

impl SelectionKeyboardState {
    pub(super) fn remember(&mut self, selection: SelectedText) {
        self.last_selection = Some(selection);
    }

    pub(super) fn copy_last_selection(&mut self) -> anyhow::Result<Option<usize>> {
        let Some(text) = self.last_selection_text() else {
            return Ok(None);
        };
        self.copy_text(&text.to_string())
    }

    pub(super) fn copy_text(&mut self, text: &str) -> anyhow::Result<Option<usize>> {
        if copy_selection(&mut self.clipboard, text)? {
            return Ok(Some(text.chars().count()));
        }
        Ok(None)
    }

    pub(super) fn paste_text(&self) -> Option<&str> {
        self.clipboard
            .text
            .as_deref()
            .filter(|text| !text.is_empty())
    }

    fn last_selection_text(&self) -> Option<&str> {
        match self.last_selection.as_ref()? {
            SelectedText::Transcript(text) | SelectedText::Composer(text) => Some(text.as_str()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TextClipboardSink {
    text: Option<String>,
}

impl ClipboardSink for TextClipboardSink {
    fn write_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.text = Some(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_requires_a_remembered_selection() {
        let mut state = SelectionKeyboardState::default();

        assert_eq!(state.copy_last_selection().unwrap(), None);
        assert_eq!(state.paste_text(), None);
    }

    #[test]
    fn copied_selection_is_available_for_paste() {
        let mut state = SelectionKeyboardState::default();

        state.remember(SelectedText::Transcript("selected text".to_string()));

        assert_eq!(state.copy_last_selection().unwrap(), Some(13));
        assert_eq!(state.paste_text(), Some("selected text"));
    }

    #[test]
    fn explicit_text_copy_is_available_for_paste() {
        let mut state = SelectionKeyboardState::default();

        assert_eq!(
            state.copy_text("https://example.com").unwrap(),
            Some("https://example.com".len())
        );
        assert_eq!(state.paste_text(), Some("https://example.com"));
    }
}
