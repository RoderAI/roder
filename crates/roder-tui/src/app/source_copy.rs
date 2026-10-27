use ratatui::widgets::Padding;

use crate::transcript::source_copy::{SourceCopyAction, SourceCopyMode, preview_lines};

use super::*;

const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct SourceCopyUi {
    mode: Option<SourceCopyMode>,
    thread_id: Option<String>,
    epoch: u64,
    pending: Option<PendingClipboardCopy>,
}

struct PendingClipboardCopy {
    thread_id: String,
    mode_epoch: Option<u64>,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Drop for SourceCopyUi {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.handle.abort();
        }
    }
}

impl<C: AppClient> TuiApp<C> {
    pub(super) fn source_copy_active(&self) -> bool {
        self.source_copy.mode.is_some()
            && self.source_copy.thread_id.as_deref() == Some(self.focused_thread_id())
    }

    pub(super) fn begin_source_copy(&mut self) {
        if self.source_copy.pending.is_some() {
            self.push_event("A clipboard copy is still in progress.".to_owned());
            return;
        }
        let mut mode = SourceCopyMode::default();
        self.timeline.sync_source_copy(&mut mode);
        if mode.is_empty() {
            self.push_event("No assistant response to copy yet.".to_owned());
            return;
        }
        self.source_copy.epoch = self.source_copy.epoch.wrapping_add(1);
        self.source_copy.thread_id = Some(self.focused_thread_id().to_owned());
        self.source_copy.mode = Some(mode);
    }

    /// The caller handles approval/user-input dialogs first. All other keys are
    /// consumed while copy mode is open, so typing cannot submit a prompt.
    pub(super) fn handle_source_copy_key(&mut self, key: KeyEvent) -> bool {
        if !self.source_copy_active() {
            self.source_copy.mode = None;
            return false;
        }
        let Some(mode) = self.source_copy.mode.as_mut() else {
            return false;
        };
        self.timeline.sync_source_copy(mode);
        match mode.handle_key(key) {
            SourceCopyAction::Close => self.source_copy.mode = None,
            SourceCopyAction::Copy => {
                let Some(text) = mode.selected_text().map(str::to_owned) else {
                    return true;
                };
                mode.copying = true;
                mode.error = None;
                self.start_clipboard_copy(text);
            }
            SourceCopyAction::Handled => {}
        }
        true
    }

    /// Also used by mouse selection, so success is never shown before the
    /// external clipboard process has actually accepted the text.
    pub(super) fn start_clipboard_copy(&mut self, text: String) {
        if self.source_copy.pending.is_some() {
            return;
        }
        self.source_copy.pending = Some(PendingClipboardCopy {
            thread_id: self.focused_thread_id().to_owned(),
            mode_epoch: self
                .source_copy
                .mode
                .as_ref()
                .map(|_| self.source_copy.epoch),
            handle: tokio::spawn(async move {
                tokio::time::timeout(CLIPBOARD_TIMEOUT, copy_selection_to_clipboards(text))
                    .await
                    .map_err(|_| anyhow::anyhow!("clipboard command timed out"))?
            }),
        });
    }

    pub(super) async fn poll_source_copy_completion(&mut self) -> bool {
        if !self
            .source_copy
            .pending
            .as_ref()
            .is_some_and(|pending| pending.handle.is_finished())
        {
            return false;
        }
        let pending = self
            .source_copy
            .pending
            .take()
            .expect("finished copy exists");
        let result = match pending.handle.await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("clipboard task failed: {error}")),
        };
        // A completion from the previous thread must not close a new selection
        // or publish a misleading helper into the current conversation.
        if pending.thread_id != self.focused_thread_id() {
            return true;
        }
        let same_mode = pending.mode_epoch == Some(self.source_copy.epoch);
        match result {
            Ok(()) => {
                if same_mode {
                    self.source_copy.mode = None;
                }
                self.copied_helper = Some(CopiedHelper {
                    shown_at: Instant::now(),
                });
                self.push_event("selection copied".to_owned());
            }
            Err(error) => {
                let message = format!("Clipboard copy failed: {error}");
                if same_mode && let Some(mode) = &mut self.source_copy.mode {
                    mode.copying = false;
                    mode.error = Some(message.clone());
                }
                self.push_event(message);
            }
        }
        true
    }

    pub(super) fn render_source_copy(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if !self.source_copy_active() {
            self.source_copy.mode = None;
            return;
        }
        let Some(mode) = self.source_copy.mode.as_mut() else {
            return;
        };
        self.timeline.sync_source_copy(mode);
        render_source_copy_mode(frame, area, mode, self.theme);
    }
}

fn source_copy_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(1, 120).min(area.width);
    let height = area.height.saturating_sub(4).clamp(1, 32).min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// This `#dialog` uses the existing surface, border, accent, text,
/// muted and error semantic roles. Preview rendering never mutates copy bytes.
fn render_source_copy_mode(frame: &mut Frame<'_>, area: Rect, mode: &SourceCopyMode, theme: Theme) {
    let popup = source_copy_area(area);
    if popup.is_empty() {
        return;
    }
    frame.render_widget(Clear, popup);
    let title = match mode.selected_position() {
        Some((position, total)) => format!(" Copy source {position}/{total} "),
        None => " Copy source ".to_owned(),
    };
    let block = Block::default()
        .borders(if theme.borders_visible {
            Borders::ALL
        } else {
            Borders::NONE
        })
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(1))
        .title(Span::styled(title, theme.accent()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let label = preview_lines(&mode.selected_label()).join(" ");
    frame.render_widget(Paragraph::new(label).style(theme.strong()), chunks[0]);
    let source = mode.selected_text().unwrap_or_default();
    let preview = source_preview_rows(source, chunks[1].width);
    let max_scroll = preview
        .len()
        .saturating_sub(usize::from(chunks[1].height))
        .min(usize::from(u16::MAX)) as u16;
    let preview = Paragraph::new(Text::from(preview)).style(theme.text());
    frame.render_widget(
        preview.scroll((mode.preview_scroll().min(max_scroll), 0)),
        chunks[1],
    );
    let status = if let Some(error) = &mode.error {
        Line::styled(error.clone(), theme.error())
    } else if mode.copying {
        Line::styled("Copying to clipboard...", theme.muted())
    } else {
        Line::styled(
            "Original source; display wrapping is not copied.",
            theme.muted(),
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::styled(
                "j/k or arrows: target  Home/End: first/last  PgUp/PgDn: scroll  Enter/y: copy  Esc: close",
                theme.muted(),
            ),
        ]),
        chunks[2],
    );
}

/// Wrap explicit display rows without trimming whitespace. Paragraph's default
/// clipping then has exactly the same row model as the scroll clamp.
fn source_preview_rows(source: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let mut rows = Vec::new();
    for text in preview_lines(source) {
        let line = Line::raw(text);
        let mut row = String::new();
        let mut row_width = 0;
        for grapheme in line.styled_graphemes(Style::default()) {
            let grapheme_width = Span::raw(grapheme.symbol).width();
            if row_width > 0 && row_width + grapheme_width > width {
                rows.push(Line::raw(std::mem::take(&mut row)));
                row_width = 0;
            }
            row.push_str(grapheme.symbol);
            row_width += grapheme_width;
        }
        rows.push(Line::raw(row));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::source_copy::SourceMessageId;
    use ratatui::{Terminal, backend::TestBackend};
    use roder_core::Runtime;

    fn app() -> TuiApp {
        let server = Arc::new(AppServer::new(Arc::new(Runtime::fake().expect("runtime"))));
        super::super::tests::test_app_with_client(LocalAppClient::new(server.clone()), server)
    }

    #[test]
    fn source_preview_resize_does_not_change_identity_or_clipboard_bytes() {
        let mut mode = SourceCopyMode::default();
        let source = "\tcode with trailing space  \r\n".repeat(5);
        mode.sync([(SourceMessageId::fresh(), source.as_str())]);
        let identity = mode.selected_id();
        for (width, height) in [(120, 35), (30, 12), (8, 4), (1, 1), (90, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal
                .draw(|frame| {
                    render_source_copy_mode(frame, frame.area(), &mode, Theme::for_terminal());
                })
                .expect("draw");
            assert_eq!(mode.selected_id(), identity);
            assert_eq!(mode.selected_text(), Some(source.as_str()));
            let area = source_copy_area(Rect::new(3, 4, width, height));
            assert!(area.right() <= 3 + width);
            assert!(area.bottom() <= 4 + height);
        }
    }

    #[test]
    fn explicit_preview_rows_preserve_spaces_and_unicode_graphemes() {
        let source = "ab  cd  \r\n\tend";
        let rows = source_preview_rows(source, 4);
        let texts = rows
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, ["ab  ", "cd  ", "    ", "end"]);

        let source = "e\u{301}\u{754c}e\u{301}\u{754c}";
        let rows = source_preview_rows(source, 3);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.width() == 3));
        assert_eq!(
            rows.iter()
                .flat_map(|line| &line.spans)
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            source
        );
    }

    #[tokio::test]
    async fn clipboard_failure_keeps_selection_and_does_not_report_success() {
        let mut app = app();
        app.timeline
            .push_assistant_delta_immediate("hello  \r\n", None);
        app.begin_source_copy();
        let identity = app.source_copy.mode.as_ref().unwrap().selected_id();
        app.source_copy.mode.as_mut().unwrap().copying = true;
        app.source_copy.pending = Some(PendingClipboardCopy {
            thread_id: app.thread_id.clone(),
            mode_epoch: Some(app.source_copy.epoch),
            handle: tokio::spawn(async { anyhow::bail!("clipboard unavailable") }),
        });
        tokio::task::yield_now().await;
        assert!(app.poll_source_copy_completion().await);
        assert!(app.copied_helper.is_none());
        let mode = app.source_copy.mode.as_ref().unwrap();
        assert_eq!(mode.selected_id(), identity);
        assert_eq!(mode.selected_text(), Some("hello  \r\n"));
        assert!(
            mode.error
                .as_ref()
                .unwrap()
                .contains("clipboard unavailable")
        );
        assert!(!mode.copying);
    }

    #[tokio::test]
    async fn success_is_acknowledged_only_after_clipboard_completion() {
        let mut app = app();
        app.timeline.push_assistant_delta_immediate("hello", None);
        app.begin_source_copy();
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.source_copy.pending = Some(PendingClipboardCopy {
            thread_id: app.thread_id.clone(),
            mode_epoch: Some(app.source_copy.epoch),
            handle: tokio::spawn(async move {
                rx.await?;
                Ok(())
            }),
        });
        assert!(!app.poll_source_copy_completion().await);
        assert!(app.copied_helper.is_none());
        assert!(app.source_copy_active());
        tx.send(()).unwrap();
        tokio::task::yield_now().await;
        assert!(app.poll_source_copy_completion().await);
        assert!(!app.source_copy_active());
        assert!(app.copied_helper.is_some());
    }

    #[test]
    fn source_mode_consumes_typing_and_dismisses_without_changing_composer() {
        let mut app = app();
        app.timeline.push_assistant_delta_immediate("hello", None);
        app.begin_source_copy();
        assert!(app.handle_source_copy_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
        assert!(app.source_copy_active());
        assert!(app.handle_source_copy_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.source_copy_active());
        assert!(!app.handle_source_copy_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
    }

    #[tokio::test]
    async fn previous_thread_clipboard_completion_cannot_close_current_mode() {
        let mut app = app();
        app.timeline.push_assistant_delta_immediate("hello", None);
        app.begin_source_copy();
        app.source_copy.pending = Some(PendingClipboardCopy {
            thread_id: "previous-thread".to_owned(),
            mode_epoch: Some(app.source_copy.epoch),
            handle: tokio::spawn(async { Ok(()) }),
        });
        tokio::task::yield_now().await;
        assert!(app.poll_source_copy_completion().await);
        assert!(app.source_copy_active());
        assert!(app.copied_helper.is_none());
    }

    #[test]
    fn changing_threads_dismisses_copy_mode_without_consuming_new_thread_input() {
        let mut app = app();
        app.timeline.push_assistant_delta_immediate("hello", None);
        app.begin_source_copy();
        app.thread_id = "another-thread".to_owned();
        assert!(!app.source_copy_active());
        assert!(!app.handle_source_copy_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
    }

    #[test]
    fn source_mode_tracks_focused_thread_not_only_lead_thread() {
        let mut app = app();
        app.timeline.push_assistant_delta_immediate("hello", None);
        app.roadmap_mode = Some(RoadmapModeState::new(None));
        app.roadmap_mode.as_mut().unwrap().selected_thread_id = Some("focused-child".to_owned());
        app.begin_source_copy();
        assert_eq!(app.source_copy.thread_id.as_deref(), Some("focused-child"));
        assert!(app.source_copy_active());
        // The lead thread stays unchanged while focus moves to another thread.
        app.roadmap_mode.as_mut().unwrap().selected_thread_id = Some("other-child".to_owned());
        assert!(!app.source_copy_active());
        assert!(!app.handle_source_copy_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
    }
}
