use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use super::{Theme, centered_rect};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum FooterShortcutContext {
    ComposerIdle,
    ComposerRunning,
    Timeline,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct Shortcut {
    pub keys: &'static str,
    pub action: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ShortcutGroup {
    title: &'static str,
    shortcuts: &'static [Shortcut],
}

const GLOBAL_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "?",
        action: "show shortcuts",
    },
    Shortcut {
        keys: "ctrl+p",
        action: "settings and menus",
    },
    Shortcut {
        keys: "ctrl+l",
        action: "toggle events",
    },
    Shortcut {
        keys: "shift+tab",
        action: "cycle mode",
    },
    Shortcut {
        keys: "esc",
        action: "interrupt or exit",
    },
    Shortcut {
        keys: "ctrl+c",
        action: "exit",
    },
];

const COMPOSER_IDLE_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "enter",
        action: "send",
    },
    Shortcut {
        keys: "shift+enter",
        action: "newline",
    },
    Shortcut {
        keys: "/",
        action: "commands",
    },
    Shortcut {
        keys: "tab",
        action: "timeline",
    },
    Shortcut {
        keys: "paste/drag",
        action: "attach images",
    },
    Shortcut {
        keys: "!",
        action: "shell",
    },
];

const COMPOSER_RUNNING_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "enter",
        action: "steer",
    },
    Shortcut {
        keys: "tab",
        action: "queue message",
    },
    Shortcut {
        keys: "shift+enter",
        action: "newline",
    },
    Shortcut {
        keys: "paste/drag",
        action: "attach images",
    },
    Shortcut {
        keys: "esc",
        action: "interrupt",
    },
];

const QUEUE_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "tab",
        action: "queue non-empty input during a run",
    },
    Shortcut {
        keys: "up",
        action: "edit latest queued message when input is empty",
    },
];

const TIMELINE_SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "j/k or arrows",
        action: "navigate tools",
    },
    Shortcut {
        keys: "pgup/pgdn",
        action: "scroll",
    },
    Shortcut {
        keys: "enter",
        action: "expand selected tool",
    },
    Shortcut {
        keys: "click/wheel",
        action: "select and scroll tools",
    },
    Shortcut {
        keys: "esc",
        action: "return to composer",
    },
];

const TODO_SHORTCUTS: &[Shortcut] = &[Shortcut {
    keys: "ctrl+t",
    action: "toggle todos",
}];

const HELP_DIALOG_GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Global",
        shortcuts: GLOBAL_SHORTCUTS,
    },
    ShortcutGroup {
        title: "Composer",
        shortcuts: COMPOSER_IDLE_SHORTCUTS,
    },
    ShortcutGroup {
        title: "Running Turn",
        shortcuts: COMPOSER_RUNNING_SHORTCUTS,
    },
    ShortcutGroup {
        title: "Queue",
        shortcuts: QUEUE_SHORTCUTS,
    },
    ShortcutGroup {
        title: "Timeline",
        shortcuts: TIMELINE_SHORTCUTS,
    },
    ShortcutGroup {
        title: "Todos",
        shortcuts: TODO_SHORTCUTS,
    },
];

pub(super) fn footer_hint(context: FooterShortcutContext, has_todos: bool) -> String {
    let shortcuts = match context {
        FooterShortcutContext::ComposerIdle => COMPOSER_IDLE_SHORTCUTS,
        FooterShortcutContext::ComposerRunning => COMPOSER_RUNNING_SHORTCUTS,
        FooterShortcutContext::Timeline => TIMELINE_SHORTCUTS,
    };
    let mut hint = shortcuts
        .iter()
        .map(footer_shortcut)
        .collect::<Vec<_>>()
        .join("  ");
    hint.push_str("  ? shortcuts");
    if has_todos {
        hint.push_str("  ");
        hint.push_str(&footer_shortcut(&TODO_SHORTCUTS[0]));
    }
    hint
}

pub(super) fn should_open_shortcuts_dialog(
    key: KeyEvent,
    composer_empty: bool,
    composer_focused: bool,
) -> bool {
    key.code == KeyCode::Char('?')
        && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        && composer_empty
        && composer_focused
}

pub(super) fn shortcut_dialog_close_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?'))
}

pub(super) fn render_shortcuts_dialog(f: &mut Frame<'_>, area: Rect, theme: Theme) {
    let dialog_area = centered_rect(
        area,
        area.width.saturating_sub(8).min(86).max(area.width.min(1)),
        area.height
            .saturating_sub(4)
            .min(28)
            .max(area.height.min(1)),
    );
    f.render_widget(Clear, dialog_area);

    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    let block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(" shortcuts  [?] ", theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(shortcut_lines(theme), chunks[0]);
    f.render_widget(controls_line(theme), chunks[1]);
}

fn shortcut_lines(theme: Theme) -> Paragraph<'static> {
    let mut lines = Vec::new();
    for (index, group) in HELP_DIALOG_GROUPS.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(group.title, theme.strong())));
        for shortcut in group.shortcuts {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18}", shortcut.keys), theme.dialog_key()),
                Span::styled(shortcut.action, theme.muted()),
            ]));
        }
    }

    Paragraph::new(Text::from(lines))
        .style(theme.dialog_surface())
        .wrap(Wrap { trim: false })
}

fn controls_line(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled("Esc", theme.dialog_key()),
        Span::styled(" close   ", theme.muted()),
        Span::styled("?", theme.dialog_key()),
        Span::styled(" close   ", theme.muted()),
        Span::styled("Enter", theme.dialog_key()),
        Span::styled(" close", theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn footer_shortcut(shortcut: &Shortcut) -> String {
    format!("{} {}", shortcut.keys, shortcut.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_hints_are_derived_from_shortcut_catalog() {
        let hint = footer_hint(FooterShortcutContext::ComposerIdle, true);

        assert!(hint.contains("enter send"));
        assert!(hint.contains("? shortcuts"));
        assert!(hint.contains("ctrl+t toggle todos"));
    }

    #[test]
    fn question_mark_opens_help_only_from_empty_focused_composer() {
        let question = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);

        assert!(should_open_shortcuts_dialog(question, true, true));
        assert!(!should_open_shortcuts_dialog(question, false, true));
        assert!(!should_open_shortcuts_dialog(question, true, false));
    }

    #[test]
    fn shortcut_dialog_has_close_keys() {
        assert!(shortcut_dialog_close_key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE
        )));
        assert!(shortcut_dialog_close_key(KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::SHIFT
        )));
    }
}
