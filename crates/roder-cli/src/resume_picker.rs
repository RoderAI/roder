use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType};
use roder_api::session::SessionMetadata;
use time::UtcOffset;

const MAX_VISIBLE_SESSIONS: usize = 10;
const HEADER_ROWS: u16 = 3;
const ROW_HOT_ACCENT: Color = Color::Rgb {
    r: 46,
    g: 102,
    b: 161,
};
const ROW_BG: Color = Color::Rgb {
    r: 37,
    g: 44,
    b: 64,
};

pub fn pick_session(sessions: &[SessionMetadata]) -> anyhow::Result<Option<String>> {
    if sessions.is_empty() {
        println!("No saved sessions found.");
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let (_, start_row) = crossterm::cursor::position()?;
    let raw = RawModeGuard::enter()?;
    execute!(stdout, Hide)?;
    let current_dir = std::env::current_dir()
        .ok()
        .and_then(|dir| dir.to_str().map(str::to_string));
    let mut only_current_directory = false;

    let mut query = String::new();
    let mut selected = 0usize;
    let mut matches = filtered_sessions(
        sessions,
        &query,
        only_current_directory,
        current_dir.as_deref(),
    );
    render(
        &mut stdout,
        start_row,
        &query,
        sessions,
        &matches,
        selected,
        only_current_directory,
    )?;

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
                if selected + 1 < matches.len() {
                    selected += 1;
                }
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('f'),
                modifiers,
                ..
            }) if modifiers == KeyModifiers::CONTROL => {
                only_current_directory = !only_current_directory;
                matches = filtered_sessions(
                    sessions,
                    &query,
                    only_current_directory,
                    current_dir.as_deref(),
                );
                selected = 0;
            }
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                ..
            }) => {
                query.pop();
                matches = filtered_sessions(
                    sessions,
                    &query,
                    only_current_directory,
                    current_dir.as_deref(),
                );
                selected = clamp_selection(selected, matches.len());
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            }) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                query.push(ch);
                matches = filtered_sessions(
                    sessions,
                    &query,
                    only_current_directory,
                    current_dir.as_deref(),
                );
                selected = clamp_selection(selected, matches.len());
            }
            _ => {}
        }
        selected = clamp_selection(selected, matches.len());
        render(
            &mut stdout,
            start_row,
            &query,
            sessions,
            &matches,
            selected,
            only_current_directory,
        )?;
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
    only_current_directory: bool,
) -> anyhow::Result<()> {
    let width = render_width();
    let visible = visible_session_count(matches.len(), start_row);
    let scroll_start = session_window_start(selected, matches.len(), visible);
    execute!(
        stdout,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown)
    )?;
    render_header_line(
        stdout,
        start_row,
        &format!(
            "Search saved sessions ({}): {query}",
            if only_current_directory {
                "current folder"
            } else {
                "all folders"
            }
        ),
        width,
    )?;
    render_hint_line(
        stdout,
        start_row.saturating_add(1),
        "Enter resume  Esc cancel  Ctrl+F toggle folder scope  Up/Down select",
        width,
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

    for row in 0..visible {
        let Some(index) = matches.get(scroll_start + row).copied() else {
            break;
        };
        let session = &sessions[index];
        let is_selected = index == selected;
        let mut line = session_line(session, width);
        if is_selected {
            line.insert_str(0, "> ");
        } else {
            line.insert_str(0, "  ");
        }
        let line = fit_line(&line, width);
        render_line(
            stdout,
            start_row
                .saturating_add(HEADER_ROWS)
                .saturating_add(row as u16),
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
        execute!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetBackgroundColor(ROW_HOT_ACCENT),
            SetForegroundColor(Color::White),
            SetAttribute(Attribute::Underlined),
        )?;
    } else {
        execute!(stdout, SetForegroundColor(ROW_BG),)?;
    }
    execute!(stdout, Print(fit_line(text, width)))?;
    execute!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

fn render_header_line(
    stdout: &mut io::Stdout,
    row: u16,
    text: &str,
    width: usize,
) -> anyhow::Result<()> {
    execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    execute!(
        stdout,
        SetAttribute(Attribute::Bold),
        SetForegroundColor(Color::Rgb {
            r: 130,
            g: 210,
            b: 255
        })
    )?;
    execute!(
        stdout,
        Print(format!(
            "{}{}",
            text,
            " ".repeat(width.saturating_sub(text.len()))
        ))
    )?;
    execute!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

fn render_hint_line(
    stdout: &mut io::Stdout,
    row: u16,
    text: &str,
    width: usize,
) -> anyhow::Result<()> {
    execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    execute!(
        stdout,
        SetForegroundColor(Color::Rgb {
            r: 130,
            g: 200,
            b: 180
        }),
        SetAttribute(Attribute::Italic),
    )?;
    execute!(stdout, Print(fit_line(text, width)))?;
    execute!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
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

fn clamp_selection(selected: usize, match_count: usize) -> usize {
    if match_count == 0 {
        0
    } else {
        selected.min(match_count.saturating_sub(1))
    }
}

fn session_window_start(selected: usize, match_count: usize, visible: usize) -> usize {
    if match_count <= visible {
        return 0;
    }

    let half_window = visible / 2;
    let max_start = match_count.saturating_sub(visible);
    selected.saturating_sub(half_window).min(max_start)
}

fn filtered_sessions(
    sessions: &[SessionMetadata],
    query: &str,
    only_current_directory: bool,
    current_dir: Option<&str>,
) -> Vec<usize> {
    let current_path = current_dir.and_then(|dir| Some(normalize_path_for_filter(Path::new(dir))));
    let query = query.trim().to_ascii_lowercase();
    let mut matches: Vec<_> = sessions
        .iter()
        .enumerate()
        .filter_map(|(index, session)| {
            if only_current_directory {
                if !session_in_current_directory(
                    session.workspace.as_deref(),
                    current_path.as_deref(),
                ) {
                    return None;
                }
            }
            if query.is_empty() || searchable_text(session).contains(&query) {
                Some(index)
            } else {
                None
            }
        })
        .collect();

    matches.sort_by(|left, right| sessions[*right].updated_at.cmp(&sessions[*left].updated_at));
    matches
}

fn session_in_current_directory(
    session_workspace: Option<&str>,
    current_dir: Option<&Path>,
) -> bool {
    let Some(current_dir) = current_dir else {
        return false;
    };
    let Some(session_workspace) = session_workspace else {
        return false;
    };
    let normalized_session_workspace = normalize_path_for_filter(Path::new(session_workspace));
    normalized_session_workspace.starts_with(current_dir)
}

fn normalize_path_for_filter(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let workspace = compacted_workspace(session.workspace.as_deref());
    let model = session.model.as_deref().unwrap_or("unknown model");
    let date = human_time(session.updated_at);
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
    let workspace = truncate(&workspace, workspace_width);
    let model = truncate(model, model_width);
    let right = truncate(&right, right_width.max(1));

    let line = format!(
        "{}  {}  |  {}  |  {}  |  {}",
        date, title, workspace, model, right,
    );
    fit_line(&line, width)
}

fn human_time(ts: time::OffsetDateTime) -> String {
    let local_ts = UtcOffset::current_local_offset()
        .ok()
        .map(|offset| ts.to_offset(offset))
        .unwrap_or(ts);
    local_ts
        .format(&time::format_description::parse("[year]-[month]-[day] [hour]:[minute]").unwrap())
        .unwrap_or_else(|_| local_ts.unix_timestamp().to_string())
}

fn compacted_workspace(workspace: Option<&str>) -> String {
    let workspace = workspace
        .filter(|workspace| !workspace.trim().is_empty())
        .unwrap_or("(unknown directory)");

    let Ok(home) = std::env::var("HOME") else {
        return workspace.to_string();
    };
    let Ok(workspace_path) = Path::new(workspace).canonicalize() else {
        return workspace.to_string();
    };

    if workspace_path.starts_with(&home) {
        workspace_path
            .strip_prefix(home)
            .map(|relative| format!("~{}", relative.display()))
            .unwrap_or_else(|_| workspace.to_string())
    } else {
        workspace.to_string()
    }
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
    use time::{Duration, OffsetDateTime};

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

        let no_query = filtered_sessions(&sessions, "", false, None);
        assert_eq!(no_query.len(), 2);
        assert_eq!(filtered_sessions(&sessions, "gode", false, None), vec![0]);
        assert_eq!(filtered_sessions(&sessions, "beta", false, None), vec![1]);
        assert_eq!(filtered_sessions(&sessions, "resume", false, None), vec![0]);
    }

    #[test]
    fn filters_only_current_directory() {
        let sessions = vec![
            session(
                "thread-alpha",
                Some("Fix resume menu"),
                Some("/Users/pz/w/gode"),
            ),
            session("thread-beta", Some("Other work"), Some("/tmp/example")),
        ];

        let current = Some("/Users/pz/w/gode");
        let all = filtered_sessions(&sessions, "", false, current);
        let local = filtered_sessions(&sessions, "", true, current);

        assert_eq!(all.len(), 2);
        assert_eq!(local, vec![0]);
    }

    #[test]
    fn session_line_includes_directory_model_count_and_short_id() {
        let session = session(
            "123456789abcdef",
            Some("Fix resume menu"),
            Some("/Users/pz/w/gode"),
        );

        let detail = session_line(&session, 220);

        assert!(detail.contains("gode"));
        assert!(detail.contains("mock"));
        assert!(detail.contains("2 msg"));
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
            updated_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
            message_count: 2,
        }
    }

    #[test]
    fn sorts_filtered_sessions_by_updated_at_desc() {
        let older = SessionMetadata {
            thread_id: "thread-old".to_string(),
            title: Some("Older session".to_string()),
            workspace: Some(std::env::temp_dir().to_string_lossy().to_string()),
            provider: Some("mock".to_string()),
            model: Some("mock".to_string()),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
            message_count: 1,
        };
        let newer = SessionMetadata {
            thread_id: "thread-new".to_string(),
            title: Some("Newer session".to_string()),
            workspace: Some(std::env::temp_dir().to_string_lossy().to_string()),
            provider: Some("mock".to_string()),
            model: Some("mock".to_string()),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(10),
            message_count: 1,
        };
        let sessions = vec![older, newer];

        let results = filtered_sessions(&sessions, "", false, None);
        assert_eq!(results, vec![1, 0]);
    }
}
