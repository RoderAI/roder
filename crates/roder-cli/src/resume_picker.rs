use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType};
use roder_api::session::SessionMetadata;

const MAX_VISIBLE_SESSIONS: usize = 10;
const HEADER_ROWS: u16 = 3;

pub fn pick_session(sessions: &[SessionMetadata]) -> anyhow::Result<Option<String>> {
    if sessions.is_empty() {
        println!("No saved sessions found.");
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let (_, start_row) = crossterm::cursor::position()?;
    let raw = RawModeGuard::enter()?;
    execute!(stdout, Hide)?;

    let mut query = String::new();
    let mut selected = 0usize;
    let mut matches = filtered_sessions(sessions, &query);
    render(&mut stdout, start_row, &query, sessions, &matches, selected)?;

    let picked = loop {
        match event::read()? {
            Event::Key(KeyEvent {
                code: KeyCode::Esc, ..
            }) => break None,
            Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            }) if modifiers.contains(KeyModifiers::CONTROL) => break None,
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            }) => {
                let Some(index) = matches.get(selected).copied() else {
                    continue;
                };
                break Some(sessions[index].thread_id.clone());
            }
            Event::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) => {
                selected = selected.saturating_sub(1);
            }
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) => {
                if selected + 1 < visible_session_count(matches.len(), start_row) {
                    selected += 1;
                }
            }
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                ..
            }) => {
                query.pop();
                matches = filtered_sessions(sessions, &query);
                selected = clamp_selection(selected, matches.len(), start_row);
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            }) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                query.push(ch);
                matches = filtered_sessions(sessions, &query);
                selected = clamp_selection(selected, matches.len(), start_row);
            }
            _ => {}
        }
        render(&mut stdout, start_row, &query, sessions, &matches, selected)?;
    };

    execute!(
        stdout,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown),
        Show
    )?;
    drop(raw);
    if let Some(thread_id) = picked.as_deref() {
        if let Some(session) = sessions
            .iter()
            .find(|session| session.thread_id == thread_id)
        {
            println!(
                "Resuming {} ({})",
                session_title(session),
                short_id(&session.thread_id)
            );
        }
    }
    Ok(picked)
}

fn render(
    stdout: &mut io::Stdout,
    start_row: u16,
    query: &str,
    sessions: &[SessionMetadata],
    matches: &[usize],
    selected: usize,
) -> anyhow::Result<()> {
    let width = render_width();
    let visible = visible_session_count(matches.len(), start_row);
    execute!(
        stdout,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown)
    )?;
    render_line(
        stdout,
        start_row,
        &format!("Search saved sessions: {query}"),
        width,
        false,
    )?;
    render_line(
        stdout,
        start_row.saturating_add(1),
        "Enter resume  Esc cancel  Up/Down select",
        width,
        false,
    )?;
    render_line(stdout, start_row.saturating_add(2), "", width, false)?;

    if matches.is_empty() {
        render_line(
            stdout,
            start_row.saturating_add(HEADER_ROWS),
            "No matching sessions.",
            width,
            false,
        )?;
        stdout.flush()?;
        return Ok(());
    }

    for (row, index) in matches.iter().copied().take(visible).enumerate() {
        let session = &sessions[index];
        let is_selected = row == selected;
        let mut line = session_line(session, width);
        if is_selected {
            line.insert_str(0, "> ");
        } else {
            line.insert_str(0, "  ");
        }
        let line = fit_line(&line, width);
        render_line(
            stdout,
            start_row.saturating_add(HEADER_ROWS).saturating_add(row as u16),
            &line,
            width,
            is_selected,
        )?;
    }

    if matches.len() > visible {
        let remaining = matches.len() - visible;
        render_line(
            stdout,
            start_row
                .saturating_add(HEADER_ROWS)
                .saturating_add(visible as u16)
                .saturating_add(1),
            &format!(
                "{remaining} more match{}; keep typing to narrow.",
                if remaining == 1 { "" } else { "es" }
            ),
            width,
            false,
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn render_line(
    stdout: &mut io::Stdout,
    row: u16,
    text: &str,
    width: usize,
    selected: bool,
) -> anyhow::Result<()> {
    execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    if selected {
        execute!(stdout, SetAttribute(Attribute::Reverse))?;
    }
    execute!(stdout, Print(fit_line(text, width)))?;
    if selected {
        execute!(stdout, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

fn render_width() -> usize {
    let width = terminal::size().map(|(width, _)| width).unwrap_or(80);
    usize::from(width.saturating_sub(1).max(1))
}

fn visible_session_count(match_count: usize, start_row: u16) -> usize {
    let height = terminal::size().map(|(_, height)| height).unwrap_or(24);
    let available_rows = height
        .saturating_sub(start_row)
        .saturating_sub(HEADER_ROWS)
        .saturating_sub(2);
    let visible_by_height = usize::from(available_rows.max(1));
    match_count.min(MAX_VISIBLE_SESSIONS).min(visible_by_height)
}

fn clamp_selection(selected: usize, match_count: usize, start_row: u16) -> usize {
    selected.min(visible_session_count(match_count, start_row).saturating_sub(1))
}

fn filtered_sessions(sessions: &[SessionMetadata], query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    sessions
        .iter()
        .enumerate()
        .filter_map(|(index, session)| {
            if query.is_empty() || searchable_text(session).contains(&query) {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn searchable_text(session: &SessionMetadata) -> String {
    [
        session_title(session),
        session.workspace.clone().unwrap_or_default(),
        session.provider.clone().unwrap_or_default(),
        session.model.clone().unwrap_or_default(),
        session.thread_id.clone(),
    ]
    .join(" ")
    .to_ascii_lowercase()
}

fn session_title(session: &SessionMetadata) -> String {
    session
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| format!("Session {}", short_id(&session.thread_id)))
}

fn session_line(session: &SessionMetadata, width: usize) -> String {
    let workspace = session
        .workspace
        .as_deref()
        .filter(|workspace| !workspace.trim().is_empty())
        .unwrap_or("(unknown directory)");
    let model = session.model.as_deref().unwrap_or("unknown model");
    let date = session.updated_at.date().to_string();
    let message_count = session.message_count;
    let title = session_title(session);
    let short_id = short_id(&session.thread_id);

    let reserved = 20 + 3; // date + separators.
    let free = width.saturating_sub(reserved.max(1));
    let title_width = (free.saturating_mul(40) / 100).max(12);
    let workspace_width = (free.saturating_mul(28) / 100).max(10);
    let model_width = (free.saturating_mul(18) / 100).max(8);
    let right = format!(
        "{} msg{} [{}]",
        message_count,
        if message_count == 1 { "" } else { "s" },
        short_id
    );
    let right_width = free.saturating_sub(title_width + workspace_width + model_width);

    let title = truncate(&title, title_width);
    let workspace = truncate(workspace, workspace_width);
    let model = truncate(model, model_width);
    let right = truncate(&right, right_width.max(1));

    let line = format!(
        "{}  {}  |  {}  |  {}  |  {}",
        date,
        title,
        workspace,
        model,
        right,
    );
    fit_line(&line, width)
}

fn session_detail(session: &SessionMetadata) -> String {
    let workspace = session
        .workspace
        .as_deref()
        .filter(|workspace| !workspace.trim().is_empty())
        .unwrap_or("(unknown directory)");
    let model = session.model.as_deref().unwrap_or("unknown model");
    format!(
        "{} | {} | {} message{} | {}",
        workspace,
        model,
        session.message_count,
        if session.message_count == 1 { "" } else { "s" },
        short_id(&session.thread_id)
    )
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    if max <= 3 {
        return value.chars().take(max).collect();
    }
    let mut out = value.chars().take(max - 3).collect::<String>();
    out.push_str("...");
    out
}

fn fit_line(value: &str, width: usize) -> String {
    truncate(value, width)
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> anyhow::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show);
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn filters_sessions_by_title_workspace_and_id() {
        let sessions = vec![
            session(
                "thread-alpha",
                Some("Fix resume menu"),
                Some("/Users/pz/w/gode"),
            ),
            session("thread-beta", Some("Other work"), Some("/tmp/example")),
        ];

        assert_eq!(filtered_sessions(&sessions, "gode"), vec![0]);
        assert_eq!(filtered_sessions(&sessions, "beta"), vec![1]);
        assert_eq!(filtered_sessions(&sessions, "resume"), vec![0]);
    }

    #[test]
    fn detail_includes_directory_model_count_and_short_id() {
        let session = session(
            "123456789abcdef",
            Some("Fix resume menu"),
            Some("/Users/pz/w/gode"),
        );

        let detail = session_detail(&session);

        assert!(detail.contains("/Users/pz/w/gode"));
        assert!(detail.contains("mock"));
        assert!(detail.contains("2 messages"));
        assert!(detail.contains("12345678"));
    }

    #[test]
    fn fit_line_stays_within_terminal_width() {
        let line = fit_line("abcdefghijklmnopqrstuvwxyz", 10);

        assert_eq!(line.chars().count(), 10);
        assert_eq!(line, "abcdefg...");
    }

    fn session(id: &str, title: Option<&str>, workspace: Option<&str>) -> SessionMetadata {
        SessionMetadata {
            thread_id: id.to_string(),
            title: title.map(str::to_string),
            workspace: workspace.map(str::to_string),
            provider: Some("mock".to_string()),
            model: Some("mock".to_string()),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            message_count: 2,
        }
    }
}
