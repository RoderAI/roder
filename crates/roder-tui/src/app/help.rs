use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
};

use crate::keymap::{HELP_ACTIONS, Keymap};

use super::{Theme, centered_rect};

pub(super) fn is_help_key(key: crossterm::event::KeyEvent) -> bool {
    key.code == crossterm::event::KeyCode::Char('?')
        && !key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
}

pub(super) fn render_keymap_help(f: &mut Frame<'_>, area: Rect, keymap: &Keymap, theme: Theme) {
    let help_area = centered_rect(area, area.width.min(76), area.height.min(20));
    let shadow_area = shadow_rect(help_area, area);
    f.render_widget(Clear, shadow_area);
    f.render_widget(Paragraph::new("").style(theme.dialog_shadow()), shadow_area);
    f.render_widget(Clear, help_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(" Keyboard help ", theme.accent()));
    let inner = block.inner(help_area);
    f.render_widget(block, help_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Mouse actions always have keyboard equivalents.",
            theme.accent_soft(),
        )))
        .style(theme.dialog_surface()),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Text::from(help_lines(keymap)))
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            key_span("?", theme),
            Span::styled(" / ", theme.muted()),
            key_span("Esc", theme),
            Span::styled(" Close", theme.muted()),
        ]))
        .style(theme.dialog_surface()),
        chunks[2],
    );
}

fn help_lines(keymap: &Keymap) -> Vec<Line<'static>> {
    HELP_ACTIONS
        .iter()
        .filter_map(|action| {
            let bindings = keymap.binding_labels_for(*action);
            (!bindings.is_empty()).then(|| {
                Line::from(vec![
                    Span::raw(format!("{:<24}", action.label())),
                    Span::raw(bindings.join(", ")),
                ])
            })
        })
        .collect()
}

fn key_span(label: &'static str, theme: Theme) -> Span<'static> {
    Span::styled(format!("[{label}]"), theme.dialog_key())
}

fn shadow_rect(dialog_area: Rect, bounds: Rect) -> Rect {
    Rect {
        x: dialog_area.x.saturating_add(2).min(bounds.right()),
        y: dialog_area.y.saturating_add(1).min(bounds.bottom()),
        width: dialog_area.width.min(
            bounds
                .right()
                .saturating_sub(dialog_area.x.saturating_add(2)),
        ),
        height: dialog_area.height.min(
            bounds
                .bottom()
                .saturating_sub(dialog_area.y.saturating_add(1)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn question_mark_opens_help() {
        assert!(is_help_key(KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::NONE
        )));
        assert!(!is_help_key(KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn help_lines_include_default_selection_bindings() {
        let lines = help_lines(&Keymap::default())
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(lines.iter().any(|line| line.contains("Copy selection")));
        assert!(lines.iter().any(|line| line.contains("Ctrl+Shift+C")));
        assert!(lines.iter().any(|line| line.contains("Paste to composer")));
        assert!(lines.iter().any(|line| line.contains("Ctrl+Shift+V")));
    }
}
