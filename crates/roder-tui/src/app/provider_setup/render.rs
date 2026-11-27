use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::{Action, Model, SetupStatus, Theme};

pub(super) fn draw(frame: &mut Frame<'_>, model: &Model, theme: Theme) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    // Reuse the existing dialog/dialog-surface semantic theme surfaces.
    let block = Block::default()
        .style(theme.dialog_surface())
        .border_style(theme.dialog())
        .border_type(theme.border_type)
        .borders(if theme.borders_visible {
            Borders::ALL
        } else {
            Borders::NONE
        })
        .title(Span::styled(" Google Antigravity setup ", theme.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(content(model, theme))
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: true })
            .scroll((model.scroll, 0)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(action_label(model), theme.dialog_key())),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if model.busy.is_some() {
                "Esc / Ctrl+C cancel | Up/Down scroll"
            } else {
                "Esc / Ctrl+C cancel | r: recheck | Up/Down scroll"
            },
            theme.muted(),
        )),
        chunks[2],
    );
}

fn action_label(model: &Model) -> &'static str {
    match (model.busy, model.retry, model.check.status) {
        (Some(Action::Install), _, _) => "Installing... Esc cancels",
        (Some(_), _, _) => "Checking account...",
        (None, Some(Action::Install), _) => "Enter: retry installation",
        (None, Some(Action::SignIn), _) => "Enter: retry Google sign-in",
        (None, Some(_), _) => "Enter: retry check",
        (None, None, SetupStatus::MissingRuntime) => "Enter: install | Esc: not now",
        (None, None, SetupStatus::NeedsSignIn) => "Enter: Google sign-in",
        (None, None, SetupStatus::Ready) => "Enter: continue to Roder",
    }
}

fn content(model: &Model, theme: Theme) -> Text<'static> {
    let (heading, explanation) = match (model.busy, model.check.status) {
        (Some(Action::Install), _) => (
            "Installing the official runtime",
            "Downloading and verifying Google's runtime in your user directory.\n\
             Escape cancels setup. No sudo or system PATH changes.",
        ),
        (Some(_), _) => (
            "Checking runtime and Google account",
            "Please wait while Roder checks subscription access.",
        ),
        (None, SetupStatus::MissingRuntime) => (
            "1. Install the official runtime",
            "Roder needs Google's Antigravity runtime (agy) to use your subscription.\n\
             Enter downloads Google's runtime into your user directory. No sudo or system PATH changes.\n\
             Allow at least 1.25 GiB of disk space for download and extraction.\n\
             Nothing is downloaded until you choose Install.",
        ),
        (None, SetupStatus::NeedsSignIn) => (
            "2. Connect your Google account",
            "Sign in with the Google account that has your Antigravity subscription.\n\
             Enter opens the official runtime's sign-in in this terminal. Follow its Google URL/code flow, \
             then return here. Roder does not read or store your sign-in code.\n\
             No Gemini API key is needed. Account access is checked after sign-in.",
        ),
        (None, SetupStatus::Ready) => (
            "3. Ready",
            "The runtime and Google account are ready. Press Enter to start Roder.",
        ),
    };
    let mut lines = vec![
        Line::from(Span::styled(heading, theme.strong())),
        Line::default(),
    ];
    // Keep changing progress and failures above explanatory copy on small screens.
    if !model.progress.is_empty() {
        lines.push(Line::from(Span::styled(
            model.progress.clone(),
            theme.running(),
        )));
        lines.push(Line::default());
    }
    if let Some(error) = &model.error {
        lines.push(Line::from(Span::styled(
            "Setup needs attention",
            theme.error(),
        )));
        lines.extend(
            error
                .lines()
                .map(|line| Line::from(Span::styled(line.to_owned(), theme.error()))),
        );
        lines.push(Line::from(Span::styled(
            "Retry below, or cancel and run setup later.",
            theme.muted(),
        )));
        lines.push(Line::default());
    }
    lines.extend(
        explanation
            .lines()
            .map(|line| Line::from(Span::styled(line.to_owned(), theme.text()))),
    );
    if !model.check.detail.is_empty() {
        lines.push(Line::default());
        lines.extend(
            model
                .check
                .detail
                .lines()
                .map(|line| Line::from(Span::styled(line.to_owned(), theme.muted()))),
        );
    }
    Text::from(lines)
}
