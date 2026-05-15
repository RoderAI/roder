use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
};

use super::{ConfirmDialog, Theme, centered_rect};

struct DialogCopy {
    title: &'static str,
    context: &'static str,
    heading: &'static str,
    detail: &'static str,
    confirm_label: &'static str,
}

pub(super) fn render_confirm_dialog(
    f: &mut Frame<'_>,
    area: Rect,
    dialog: ConfirmDialog,
    theme: Theme,
) {
    let dialog_area = centered_rect(area, dialog_width(area), 8.min(area.height));
    let shadow_area = shadow_rect(dialog_area, area);
    let copy = dialog_copy(dialog);

    f.render_widget(Clear, shadow_area);
    f.render_widget(Paragraph::new("").style(theme.dialog_shadow()), shadow_area);
    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(format!(" {} ", copy.title), theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(context_line(copy.context, theme), chunks[0]);
    f.render_widget(heading_line(copy.heading, theme), chunks[1]);
    f.render_widget(detail_text(copy.detail, theme), chunks[2]);
    f.render_widget(action_line(copy.confirm_label, theme), chunks[3]);
}

fn context_line(context: &str, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        context.to_string(),
        theme.accent_soft(),
    )))
    .style(theme.dialog_surface())
}

fn heading_line(heading: &str, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        heading.to_string(),
        theme.strong(),
    )))
    .style(theme.dialog_surface())
}

fn detail_text(detail: &str, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Text::from(Line::from(Span::styled(
        detail.to_string(),
        theme.muted(),
    ))))
    .style(theme.dialog_surface())
    .wrap(Wrap { trim: true })
}

fn action_line(confirm_label: &str, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        key_chip("Enter", theme),
        Span::styled(" / ", theme.muted()),
        key_chip("Y", theme),
        Span::styled(format!(" {confirm_label}"), theme.accent()),
        Span::styled("    ", theme.dialog_surface()),
        key_chip("Esc", theme),
        Span::styled(" / ", theme.muted()),
        key_chip("N", theme),
        Span::styled(" Cancel", theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn key_chip(label: &'static str, theme: Theme) -> Span<'static> {
    Span::styled(format!("[{label}]"), theme.dialog_key())
}

fn dialog_copy(dialog: ConfirmDialog) -> DialogCopy {
    match dialog {
        ConfirmDialog::Interrupt => DialogCopy {
            title: "Interrupt turn",
            context: "running model",
            heading: "Stop the current response?",
            detail: "Roder will ask the provider to stop this turn. The session stays open and any partial output remains visible.",
            confirm_label: "Interrupt",
        },
        ConfirmDialog::Exit => DialogCopy {
            title: "Exit Roder",
            context: "terminal session",
            heading: "Close the TUI?",
            detail: "Roder will leave the alternate screen and return you to the terminal.",
            confirm_label: "Exit",
        },
    }
}

fn dialog_width(area: Rect) -> u16 {
    let roomy_width = area.width.saturating_sub(4).min(64);
    roomy_width.max(area.width.min(44))
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

    #[test]
    fn interrupt_dialog_copy_is_actionable() {
        let copy = dialog_copy(ConfirmDialog::Interrupt);

        assert_eq!(copy.title, "Interrupt turn");
        assert_eq!(copy.confirm_label, "Interrupt");
        assert!(copy.heading.contains("Stop"));
        assert!(copy.detail.contains("session stays open"));
    }

    #[test]
    fn dialog_width_keeps_margin_when_roomy() {
        assert_eq!(
            dialog_width(Rect::new(0, 0, 118, 40)),
            64,
            "wide terminals should not produce a stretched dialog"
        );
    }

    #[test]
    fn key_chip_uses_the_dialog_key_style() {
        let theme = Theme::for_dark_background(true);
        let chip = key_chip("Enter", theme);

        assert_eq!(chip.content.as_ref(), "[Enter]");
        assert_eq!(chip.style, theme.dialog_key());
    }

    #[test]
    fn shadow_stays_inside_bounds() {
        let bounds = Rect::new(0, 0, 20, 8);
        let shadow = shadow_rect(Rect::new(2, 2, 16, 5), bounds);

        assert_eq!(shadow, Rect::new(4, 3, 16, 5));
    }
}
