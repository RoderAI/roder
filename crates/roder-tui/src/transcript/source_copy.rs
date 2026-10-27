//! Keyboard copy targets backed by original transcript text, never visual rows.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Identity belongs to the source message, not its position or rendered width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceMessageId(u64);

impl SourceMessageId {
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyTargetKind {
    Message,
    CodeBlock { opening_offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyTargetId {
    pub message: SourceMessageId,
    pub kind: CopyTargetKind,
}

#[derive(Debug, Clone)]
struct SourceMessage {
    id: SourceMessageId,
    text: String,
}

#[derive(Debug, Clone)]
struct CopyTarget {
    id: CopyTargetId,
    message_index: usize,
    range: Range<usize>,
    language: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCopyAction {
    Handled,
    Close,
    Copy,
}

/// A selection remains attached to a message/fence as streaming appends text,
/// older history is prepended, or terminal width changes.
#[derive(Debug, Default)]
pub struct SourceCopyMode {
    messages: Vec<SourceMessage>,
    targets: Vec<CopyTarget>,
    selected: Option<CopyTargetId>,
    preview_scroll: u16,
    pub error: Option<String>,
    pub copying: bool,
}

impl SourceCopyMode {
    /// Synchronize borrowed sources. Unchanged messages are not cloned/reparsed.
    pub fn sync<'a>(&mut self, sources: impl IntoIterator<Item = (SourceMessageId, &'a str)>) {
        let sources: Vec<_> = sources.into_iter().collect();
        if sources.len() == self.messages.len()
            && sources
                .iter()
                .zip(&self.messages)
                .all(|((id, text), old)| *id == old.id && *text == old.text)
        {
            return;
        }
        let previous = self.selected;
        self.messages = sources
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .map(|(id, text)| SourceMessage {
                id,
                text: text.to_owned(),
            })
            .collect();
        self.targets.clear();
        for (message_index, message) in self.messages.iter().enumerate() {
            self.targets.push(CopyTarget {
                id: CopyTargetId {
                    message: message.id,
                    kind: CopyTargetKind::Message,
                },
                message_index,
                range: 0..message.text.len(),
                language: None,
            });
            for block in fenced_blocks(&message.text) {
                self.targets.push(CopyTarget {
                    id: CopyTargetId {
                        message: message.id,
                        kind: CopyTargetKind::CodeBlock {
                            opening_offset: block.opening_offset,
                        },
                    },
                    message_index,
                    range: block.contents,
                    language: block.language,
                });
            }
        }
        self.selected = previous
            .filter(|id| self.targets.iter().any(|target| target.id == *id))
            .or_else(|| {
                previous.and_then(|id| {
                    self.targets
                        .iter()
                        .find(|target| {
                            target.id.message == id.message
                                && target.id.kind == CopyTargetKind::Message
                        })
                        .map(|target| target.id)
                })
            })
            .or_else(|| {
                // Start with the latest whole response, not its last code block.
                self.targets
                    .iter()
                    .rev()
                    .find(|target| target.id.kind == CopyTargetKind::Message)
                    .map(|target| target.id)
            });
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn selected_id(&self) -> Option<CopyTargetId> {
        self.selected
    }

    pub fn selected_text(&self) -> Option<&str> {
        let target = self.selected_target()?;
        self.messages[target.message_index]
            .text
            .get(target.range.clone())
    }

    pub fn selected_position(&self) -> Option<(usize, usize)> {
        Some((self.selected_index()? + 1, self.targets.len()))
    }

    pub fn selected_label(&self) -> String {
        let Some(target) = self.selected_target() else {
            return "No response selected".to_string();
        };
        let message = target.message_index + 1;
        match target.id.kind {
            CopyTargetKind::Message => format!("Response {message} - original Markdown"),
            CopyTargetKind::CodeBlock { .. } => match target.language.as_deref() {
                Some(language) if !language.is_empty() => {
                    format!("Response {message} - {language} source")
                }
                _ => format!("Response {message} - code source"),
            },
        }
    }

    pub fn preview_scroll(&self) -> u16 {
        self.preview_scroll
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SourceCopyAction {
        if key.kind == KeyEventKind::Release {
            return SourceCopyAction::Handled;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return SourceCopyAction::Close;
        }
        if self.copying {
            return SourceCopyAction::Handled;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => return SourceCopyAction::Copy,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.move_selection(1),
            KeyCode::Home | KeyCode::Char('g') => self.select_index(0),
            KeyCode::End | KeyCode::Char('G') => {
                self.select_index(self.targets.len().saturating_sub(1));
            }
            KeyCode::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(10),
            KeyCode::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(10),
            _ => {}
        }
        SourceCopyAction::Handled
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.targets.iter().position(|target| target.id == selected)
    }

    fn selected_target(&self) -> Option<&CopyTarget> {
        self.targets.get(self.selected_index()?)
    }

    fn move_selection(&mut self, amount: isize) {
        let index = self.selected_index().unwrap_or_default();
        self.select_index(index.saturating_add_signed(amount));
    }

    fn select_index(&mut self, index: usize) {
        self.selected = self
            .targets
            .get(index.min(self.targets.len().saturating_sub(1)))
            .map(|target| target.id);
        self.preview_scroll = 0;
        self.error = None;
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FencedBlock {
    opening_offset: usize,
    contents: Range<usize>,
    language: Option<String>,
}

struct OpenFence {
    marker: u8,
    count: usize,
    opening_offset: usize,
    body_start: usize,
    language: Option<String>,
}

/// Byte ranges deliberately retain every original content byte, including
/// trailing whitespace, mixed line endings, tabs, and missing final newlines.
/// Fence syntax alone is excluded. Unclosed streaming fences extend to EOF.
fn fenced_blocks(source: &str) -> Vec<FencedBlock> {
    let mut blocks = Vec::new();
    let mut open: Option<OpenFence> = None;
    for line in source_lines(source) {
        let text = &source[line.content.clone()];
        let Some((marker, count, info)) = fence(text) else {
            continue;
        };
        match open.as_ref() {
            Some(opening)
                if marker == opening.marker && count >= opening.count && info.trim().is_empty() =>
            {
                blocks.push(FencedBlock {
                    opening_offset: opening.opening_offset,
                    contents: opening.body_start..line.content.start,
                    language: opening.language.clone(),
                });
                open = None;
            }
            None if marker != b'`' || !info.contains('`') => {
                open = Some(OpenFence {
                    marker,
                    count,
                    opening_offset: line.content.start,
                    body_start: line.end,
                    language: info.split_whitespace().next().map(str::to_owned),
                });
            }
            _ => {}
        }
    }
    if let Some(opening) = open {
        blocks.push(FencedBlock {
            opening_offset: opening.opening_offset,
            contents: opening.body_start..source.len(),
            language: opening.language,
        });
    }
    blocks
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let marker = *body.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let count = body.bytes().take_while(|byte| *byte == marker).count();
    (count >= 3).then(|| (marker, count, &body[count..]))
}

struct SourceLine {
    content: Range<usize>,
    end: usize,
}

fn source_lines(source: &str) -> impl Iterator<Item = SourceLine> + '_ {
    let mut position = 0;
    std::iter::from_fn(move || {
        if position == source.len() {
            return None;
        }
        let start = position;
        let bytes = source.as_bytes();
        while position < bytes.len() && !matches!(bytes[position], b'\n' | b'\r') {
            position += 1;
        }
        let content_end = position;
        if position < bytes.len() {
            let cr = bytes[position] == b'\r';
            position += 1;
            if cr && bytes.get(position) == Some(&b'\n') {
                position += 1;
            }
        }
        Some(SourceLine {
            content: start..content_end,
            end: position,
        })
    })
}

/// Text-only preview lines. Control characters never become terminal commands;
/// display sanitization does not modify the source returned to the clipboard.
pub fn preview_lines(source: &str) -> Vec<String> {
    source_lines(source)
        .map(|line| {
            source[line.content]
                .chars()
                .map(|character| match character {
                    '\t' => "    ".to_owned(),
                    character if character.is_control() => "\u{fffd}".to_owned(),
                    character => character.to_string(),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn copies_original_message_and_code_without_normalizing_bytes() {
        let source =
            "> Quote  \r\n\r\n| a | b |\n|---|---|\n\r```rust\r\n\tlet x = 1;  \r\n\nlast\r```\r\n";
        let mut mode = SourceCopyMode::default();
        mode.sync([(SourceMessageId::fresh(), source)]);
        assert_eq!(mode.selected_text(), Some(source));
        mode.handle_key(key(KeyCode::Down));
        assert_eq!(mode.selected_text(), Some("\tlet x = 1;  \r\n\nlast\r"));
        assert_eq!(mode.handle_key(key(KeyCode::Enter)), SourceCopyAction::Copy);
    }

    #[test]
    fn navigation_moves_across_messages_and_blocks() {
        let first = SourceMessageId::fresh();
        let second = SourceMessageId::fresh();
        let mut mode = SourceCopyMode::default();
        mode.sync([(first, "first\n```\na\n```\n"), (second, "second")]);
        assert_eq!(mode.selected_position(), Some((3, 3)));
        mode.handle_key(key(KeyCode::Char('k')));
        assert_eq!(mode.selected_text(), Some("a\n"));
        mode.handle_key(key(KeyCode::Home));
        assert_eq!(mode.selected_text(), Some("first\n```\na\n```\n"));
        mode.handle_key(key(KeyCode::End));
        assert_eq!(mode.selected_text(), Some("second"));
        assert_eq!(mode.handle_key(key(KeyCode::Esc)), SourceCopyAction::Close);
    }

    #[test]
    fn selection_identity_survives_streaming_and_history_prepend() {
        let selected = SourceMessageId::fresh();
        let mut mode = SourceCopyMode::default();
        mode.sync([(selected, "start\n```rs\nlet ")]);
        mode.handle_key(key(KeyCode::Down));
        let identity = mode.selected_id();
        mode.handle_key(key(KeyCode::PageDown));
        mode.sync([
            (SourceMessageId::fresh(), "older response"),
            (selected, "start\n```rs\nlet x = 1;  \r\n```\n"),
            (SourceMessageId::fresh(), "new response"),
        ]);
        assert_eq!(mode.selected_id(), identity);
        assert_eq!(mode.selected_text(), Some("let x = 1;  \r\n"));
        assert_eq!(mode.preview_scroll(), 10);
    }

    #[test]
    fn fence_length_marker_and_info_are_respected() {
        let source = "````md\n```nested\nx\n```\n````\n~~~py\nx  \r\n~~~~\n";
        let blocks = fenced_blocks(source);
        assert_eq!(blocks.len(), 2);
        assert_eq!(&source[blocks[0].contents.clone()], "```nested\nx\n```\n");
        assert_eq!(&source[blocks[1].contents.clone()], "x  \r\n");
        assert_eq!(blocks[1].language.as_deref(), Some("py"));
    }

    #[test]
    fn streaming_code_without_final_newline_is_not_padded() {
        let mut mode = SourceCopyMode::default();
        mode.sync([(SourceMessageId::fresh(), "```\r\nx\t  ")]);
        mode.handle_key(key(KeyCode::Down));
        assert_eq!(mode.selected_text(), Some("x\t  "));
    }

    #[test]
    fn view_sanitization_is_separate_from_source_payload() {
        let source = "\t\x1b[31mhello  \r\nnext\rthird";
        assert_eq!(
            preview_lines(source),
            ["    \u{fffd}[31mhello  ", "next", "third"]
        );
        let mut mode = SourceCopyMode::default();
        mode.sync([(SourceMessageId::fresh(), source)]);
        assert_eq!(mode.selected_text(), Some(source));
    }

    #[test]
    fn copying_blocks_navigation_and_key_release_cannot_copy_twice() {
        let mut mode = SourceCopyMode::default();
        mode.sync([(SourceMessageId::fresh(), "a")]);
        let release =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(mode.handle_key(release), SourceCopyAction::Handled);
        mode.copying = true;
        assert_eq!(
            mode.handle_key(key(KeyCode::Enter)),
            SourceCopyAction::Handled
        );
        assert_eq!(mode.handle_key(key(KeyCode::Esc)), SourceCopyAction::Close);
    }
}
