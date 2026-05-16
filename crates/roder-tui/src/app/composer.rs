use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
};
use roder_api::policy_mode::PolicyMode;
use tui_textarea::{CursorMove, TextArea, WrapMode};

use super::{Theme, pretty_policy_mode_label};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerMode {
    Chat,
    Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerKeyAction {
    Submit,
    Edited,
    Ignored,
}

impl ComposerMode {
    pub(super) fn is_shell(self) -> bool {
        matches!(self, Self::Shell)
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Chat => "Ask Roder to work on this repo",
            Self::Shell => "Run a shell command",
        }
    }

    fn title_style(self, theme: Theme) -> Style {
        match self {
            Self::Chat => theme.muted(),
            Self::Shell => theme.shell(),
        }
    }

    fn title_spans(self, theme: Theme, policy_mode: PolicyMode) -> Option<Line<'static>> {
        let mode_label = match policy_mode {
            PolicyMode::Default => "",
            _ => pretty_policy_mode_label(policy_mode),
        };

        Some(match self {
            Self::Chat if mode_label.is_empty() => return None,
            Self::Chat => Line::from(Span::styled(
                format!(" {} ", mode_label),
                theme.policy_mode(policy_mode),
            )),
            Self::Shell if mode_label.is_empty() => {
                Line::from(Span::styled(" shell ", self.title_style(theme)))
            }
            Self::Shell => Line::from(vec![
                Span::styled(" shell ", self.title_style(theme)),
                Span::styled(format!("{} ", mode_label), theme.policy_mode(policy_mode)),
            ]),
        })
    }

    #[cfg(test)]
    fn title_text(self, policy_mode: PolicyMode) -> String {
        self.title_spans(Theme::for_dark_background(true), policy_mode)
            .unwrap_or_default()
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn border_style(self, theme: Theme, policy_mode: PolicyMode) -> Style {
        match self {
            Self::Chat => theme.policy_mode(policy_mode),
            Self::Shell => theme.policy_mode(policy_mode),
        }
    }
}

pub(super) fn composer_textarea(theme: Theme) -> TextArea<'static> {
    let mut composer = TextArea::default();
    style_composer_for_mode(
        &mut composer,
        theme,
        ComposerMode::Chat,
        PolicyMode::Default,
    );
    composer.set_wrap_mode(WrapMode::WordOrGlyph);
    composer.set_min_rows(3);
    composer.set_max_rows(8);
    composer.set_style(theme.text());
    composer.set_cursor_line_style(theme.text());
    composer.set_cursor_style(Style::default().fg(theme.text).bg(cursor_color(theme)));
    composer
}

pub(super) fn style_composer_for_current_mode(
    composer: &mut TextArea<'static>,
    theme: Theme,
    policy_mode: PolicyMode,
) {
    let mode = composer_mode(composer);
    style_composer_for_mode(composer, theme, mode, policy_mode);
}

pub(super) fn composer_mode(composer: &TextArea<'_>) -> ComposerMode {
    composer_mode_from_text(&composer_text(composer))
}

pub(super) fn composer_mode_from_text(input: &str) -> ComposerMode {
    if input.starts_with('!') {
        ComposerMode::Shell
    } else {
        ComposerMode::Chat
    }
}

pub(super) fn composer_text(composer: &TextArea<'_>) -> String {
    composer.lines().join("\n")
}

pub(super) fn shell_command_from_input(input: &str) -> Option<String> {
    let command = input.strip_prefix('!')?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

pub(super) fn handle_composer_key(composer: &mut TextArea<'_>, key: KeyEvent) -> ComposerKeyAction {
    if composer_key_inserts_newline(key) {
        composer.insert_newline();
        return ComposerKeyAction::Edited;
    }

    if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
        return ComposerKeyAction::Submit;
    }

    if key.code == KeyCode::Enter {
        return ComposerKeyAction::Ignored;
    }

    if key.modifiers.contains(KeyModifiers::SUPER)
        && let Some(action) = handle_command_key(composer, key)
    {
        return action;
    }

    if composer.input(key) {
        ComposerKeyAction::Edited
    } else {
        ComposerKeyAction::Ignored
    }
}

fn cursor_color(theme: Theme) -> Color {
    theme.muted
}

fn composer_key_inserts_newline(key: KeyEvent) -> bool {
    if key.code == KeyCode::Enter {
        return key.modifiers.contains(KeyModifiers::SHIFT);
    }

    matches!(key.code, KeyCode::Char('m' | 'M'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

fn handle_command_key(composer: &mut TextArea<'_>, key: KeyEvent) -> Option<ComposerKeyAction> {
    match key.code {
        KeyCode::Backspace => Some(action_from_modified(composer.clear())),
        KeyCode::Delete => Some(action_from_modified(composer.delete_line_by_end())),
        KeyCode::Left | KeyCode::Home => {
            composer.move_cursor(CursorMove::Head);
            Some(ComposerKeyAction::Ignored)
        }
        KeyCode::Right | KeyCode::End => {
            composer.move_cursor(CursorMove::End);
            Some(ComposerKeyAction::Ignored)
        }
        KeyCode::Up => {
            composer.move_cursor(CursorMove::Top);
            Some(ComposerKeyAction::Ignored)
        }
        KeyCode::Down => {
            composer.move_cursor(CursorMove::Bottom);
            Some(ComposerKeyAction::Ignored)
        }
        _ => Some(ComposerKeyAction::Ignored),
    }
}

fn action_from_modified(modified: bool) -> ComposerKeyAction {
    if modified {
        ComposerKeyAction::Edited
    } else {
        ComposerKeyAction::Ignored
    }
}

fn style_composer_for_mode(
    composer: &mut TextArea<'static>,
    theme: Theme,
    mode: ComposerMode,
    policy_mode: PolicyMode,
) {
    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    // Inherit the body background on the border glyphs so the composer
    // frame blends into the body fill instead of stamping the terminal
    // default through each corner cell.
    let mut border_style = mode.border_style(theme, policy_mode);
    if let Some(bg) = theme.body_background {
        border_style = border_style.bg(bg);
    }
    let mut block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(border_style);

    if let Some(title) = mode.title_spans(theme, policy_mode) {
        block = block.title(title);
    }

    composer.set_block(block);
    composer.set_placeholder_text(mode.placeholder());
    composer.set_placeholder_style(mode.title_style(theme).add_modifier(Modifier::ITALIC));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;

    #[test]
    fn composer_cursor_uses_gray_instead_of_selection_pink() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);

            assert_eq!(cursor_color(theme), theme.muted);
            assert_ne!(cursor_color(theme), theme.selection_bg);
            assert!(matches!(cursor_color(theme), Color::Indexed(240 | 244)));
        }
    }

    #[test]
    fn default_chat_mode_has_no_composer_title() {
        assert_eq!(ComposerMode::Chat.title_text(PolicyMode::Default), "");
    }

    #[test]
    fn non_default_chat_mode_title_only_shows_policy_mode() {
        assert_eq!(ComposerMode::Chat.title_text(PolicyMode::Plan), " Plan ");
        assert_eq!(
            ComposerMode::Chat.title_text(PolicyMode::AcceptEdits),
            " Accept Edits "
        );
    }

    #[test]
    fn shell_mode_title_is_not_labeled_as_chat() {
        assert_eq!(
            ComposerMode::Shell.title_text(PolicyMode::Plan),
            " shell Plan "
        );
    }

    #[test]
    fn shift_enter_inserts_newline_instead_of_submitting() {
        let mut composer = TextArea::default();
        composer.insert_str("first");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
            ),
            ComposerKeyAction::Edited
        );

        assert_eq!(composer_text(&composer), "first\n");
    }

    #[test]
    fn ctrl_shift_m_inserts_newline_for_shift_enter_encodings() {
        let mut composer = TextArea::default();
        composer.insert_str("first");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(
                    KeyCode::Char('M'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                )
            ),
            ComposerKeyAction::Edited
        );

        assert_eq!(composer_text(&composer), "first\n");
    }

    #[test]
    fn enter_without_shift_submits() {
        let mut composer = TextArea::default();
        composer.insert_str("send me");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            ComposerKeyAction::Submit
        );
        assert_eq!(composer_text(&composer), "send me");
    }

    #[test]
    fn modified_enter_without_shift_does_not_submit() {
        let mut composer = TextArea::default();
        composer.insert_str("send me");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)
            ),
            ComposerKeyAction::Ignored
        );
        assert_eq!(composer_text(&composer), "send me");
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut composer = TextArea::default();
        composer.insert_str("hello world");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
            ),
            ComposerKeyAction::Edited
        );

        assert_eq!(composer_text(&composer), "hello ");
    }

    #[test]
    fn command_backspace_clears_the_composer() {
        let mut composer = TextArea::default();
        composer.insert_str("first line\nsecond line");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER)
            ),
            ComposerKeyAction::Edited
        );

        assert_eq!(composer_text(&composer), "");
    }

    #[test]
    fn command_delete_deletes_to_the_end_of_the_line() {
        let mut composer = TextArea::default();
        composer.insert_str("prefix suffix");
        for _ in 0..7 {
            composer.move_cursor(CursorMove::Back);
        }

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Delete, KeyModifiers::SUPER)
            ),
            ComposerKeyAction::Edited
        );

        assert_eq!(composer_text(&composer), "prefix");
    }

    #[test]
    fn unhandled_command_characters_do_not_insert_text() {
        let mut composer = TextArea::default();
        composer.insert_str("keep");

        assert_eq!(
            handle_composer_key(
                &mut composer,
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SUPER)
            ),
            ComposerKeyAction::Ignored
        );

        assert_eq!(composer_text(&composer), "keep");
    }
}
