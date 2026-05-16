use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::{TextArea, WrapMode};

use super::Theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerMode {
    Chat,
    Shell,
}

impl ComposerMode {
    pub(super) fn is_shell(self) -> bool {
        matches!(self, Self::Shell)
    }

    fn title(self) -> &'static str {
        match self {
            Self::Chat => " chat ",
            Self::Shell => " shell ",
        }
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

    fn border_style(self, theme: Theme) -> Style {
        match self {
            Self::Chat => theme.border(),
            Self::Shell => theme.shell(),
        }
    }
}

pub(super) fn composer_textarea(theme: Theme) -> TextArea<'static> {
    let mut composer = TextArea::default();
    style_composer_for_mode(&mut composer, theme, ComposerMode::Chat);
    composer.set_wrap_mode(WrapMode::WordOrGlyph);
    composer.set_min_rows(3);
    composer.set_max_rows(8);
    composer.set_style(theme.text());
    composer.set_cursor_line_style(theme.text());
    composer.set_cursor_style(Style::default().fg(theme.text).bg(cursor_color(theme)));
    composer
}

pub(super) fn style_composer_for_current_mode(composer: &mut TextArea<'static>, theme: Theme) {
    let mode = composer_mode(composer);
    style_composer_for_mode(composer, theme, mode);
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

fn cursor_color(theme: Theme) -> Color {
    theme.muted
}

fn style_composer_for_mode(composer: &mut TextArea<'static>, theme: Theme, mode: ComposerMode) {
    composer.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(mode.border_style(theme))
            .title(Span::styled(mode.title(), mode.title_style(theme))),
    );
    composer.set_placeholder_text(mode.placeholder());
    composer.set_placeholder_style(mode.title_style(theme).add_modifier(Modifier::ITALIC));
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
