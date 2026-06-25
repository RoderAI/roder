use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use super::{
    ConfirmChoice, ConfirmDialog, ConfirmDialogState, Theme, UserInputDialogState, centered_rect,
};

struct DialogCopy {
    title: String,
    context: String,
    heading: String,
    detail: String,
    confirm_label: String,
}

pub(super) fn render_confirm_dialog(
    f: &mut Frame<'_>,
    area: Rect,
    state: ConfirmDialogState,
    theme: Theme,
) {
    let dialog_area = centered_rect(area, dialog_width(area), 9.min(area.height));
    let shadow_area = shadow_rect(dialog_area, area);
    let copy = dialog_copy(&state.dialog);

    f.render_widget(Clear, shadow_area);
    f.render_widget(Paragraph::new("").style(theme.dialog_shadow()), shadow_area);
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
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(context_line(&copy.context, theme), chunks[0]);
    f.render_widget(heading_line(&copy.heading, theme), chunks[1]);
    f.render_widget(detail_text(&copy.detail, theme), chunks[2]);
    f.render_widget(
        action_line(&copy.confirm_label, state.selected, theme),
        chunks[3],
    );
    f.render_widget(controls_line(theme), chunks[4]);
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

fn action_line(confirm_label: &str, selected: ConfirmChoice, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        choice_chip("Yes", selected == ConfirmChoice::Yes, theme),
        Span::styled(format!(" {confirm_label}  "), theme.accent()),
        Span::styled("    ", theme.dialog_surface()),
        choice_chip("No", selected == ConfirmChoice::No, theme),
        Span::styled(" Cancel", theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn controls_line(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        key_hint("Left", theme),
        Span::styled(" / ", theme.muted()),
        key_hint("Right", theme),
        Span::styled(" select   ", theme.muted()),
        key_hint("Enter", theme),
        Span::styled(" choose   ", theme.muted()),
        key_hint("Y", theme),
        Span::styled("/", theme.muted()),
        key_hint("N", theme),
    ]))
    .style(theme.dialog_surface())
}

fn choice_chip(label: &'static str, selected: bool, theme: Theme) -> Span<'static> {
    let style = if selected {
        theme.dialog_key()
    } else {
        theme.muted()
    };
    let marker = if selected { ">" } else { " " };

    Span::styled(format!("{marker} {label} "), style)
}

fn key_hint(label: &'static str, theme: Theme) -> Span<'static> {
    Span::styled(label.to_string(), theme.dialog_key())
}

fn dialog_copy(dialog: &ConfirmDialog) -> DialogCopy {
    match dialog {
        ConfirmDialog::Interrupt => DialogCopy {
            title: "Interrupt turn".to_string(),
            context: "running model".to_string(),
            heading: "Stop the current response?".to_string(),
            detail: "Roder will ask the provider to stop this turn. The thread stays open and any partial output remains visible.".to_string(),
            confirm_label: "Interrupt".to_string(),
        },
        ConfirmDialog::Exit => DialogCopy {
            title: "Exit Roder".to_string(),
            context: "terminal session".to_string(),
            heading: "Close the TUI?".to_string(),
            detail: "Roder will leave the alternate screen and return you to the terminal."
                .to_string(),
            confirm_label: "Exit".to_string(),
        },
        ConfirmDialog::ToolApproval {
            tool_name, reason, ..
        } => DialogCopy {
            title: "Approve tool".to_string(),
            context: "tool approval".to_string(),
            heading: format!("Run `{tool_name}`?"),
            detail: reason
                .clone()
                .unwrap_or_else(|| "Roder wants to run a side-effecting tool.".to_string()),
            confirm_label: "Approve".to_string(),
        },
    }
}

/// Renders the interactive `request_user_input` selection modal: the current
/// question plus its options, with the highlighted option called out and a
/// progress hint when more than one question is pending.
pub(super) fn render_user_input_dialog(
    f: &mut Frame<'_>,
    area: Rect,
    state: &UserInputDialogState,
    theme: Theme,
) {
    let question = state.current_question();
    // Each option occupies two lines (label + wrapped description); leave room
    // for the context, heading, blank spacer, and controls lines plus borders.
    let option_lines = question.options.len() as u16 * 2;
    let desired = option_lines + 6;
    let height = desired.min(area.height.saturating_sub(2)).max(7);
    let dialog_area = centered_rect(area, dialog_width(area), height);
    let shadow_area = shadow_rect(dialog_area, area);

    f.render_widget(Clear, shadow_area);
    f.render_widget(Paragraph::new("").style(theme.dialog_shadow()), shadow_area);
    f.render_widget(Clear, dialog_area);

    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    let title = if state.questions.len() > 1 {
        format!(
            " Your input ({}/{}) ",
            state.current + 1,
            state.questions.len()
        )
    } else {
        " Your input ".to_string()
    };
    let block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(title, theme.accent()));
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

    let context = if question.header.trim().is_empty() {
        "model needs your input".to_string()
    } else {
        question.header.clone()
    };
    f.render_widget(context_line(&context, theme), chunks[0]);
    f.render_widget(heading_line(&question.question, theme), chunks[1]);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, option) in question.options.iter().enumerate() {
        let selected = index == state.selected;
        let marker = if selected { "›" } else { " " };
        let label_style = if selected {
            theme.accent()
        } else {
            theme.dialog_surface()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} {}. ", index + 1), theme.accent_soft()),
            Span::styled(option.label.clone(), label_style),
        ]));
        if !option.description.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    {}", option.description),
                theme.dialog_surface(),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .style(theme.dialog_surface()),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ move · 1-9 jump · Enter select · Esc skip".to_string(),
            theme.muted(),
        )))
        .style(theme.dialog_surface()),
        chunks[3],
    );
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
        let copy = dialog_copy(&ConfirmDialog::Interrupt);

        assert_eq!(copy.title, "Interrupt turn");
        assert_eq!(copy.confirm_label, "Interrupt");
        assert!(copy.heading.contains("Stop"));
        assert!(copy.detail.contains("thread stays open"));
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
    fn selected_choice_chip_uses_the_dialog_key_style() {
        let theme = Theme::for_dark_background(true);
        let chip = choice_chip("Yes", true, theme);

        assert_eq!(chip.content.as_ref(), "> Yes ");
        assert_eq!(chip.style, theme.dialog_key());
    }

    #[test]
    fn unselected_choice_chip_is_muted() {
        let theme = Theme::for_dark_background(true);
        let chip = choice_chip("No", false, theme);

        assert_eq!(chip.content.as_ref(), "  No ");
        assert_eq!(chip.style, theme.muted());
    }

    #[test]
    fn shadow_stays_inside_bounds() {
        let bounds = Rect::new(0, 0, 20, 8);
        let shadow = shadow_rect(Rect::new(2, 2, 16, 5), bounds);

        assert_eq!(shadow, Rect::new(4, 3, 16, 5));
    }
}
