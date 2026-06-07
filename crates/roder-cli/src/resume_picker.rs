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
use roder_protocol::{Item, Thread};
use time::UtcOffset;

const MAX_VISIBLE_THREADS: usize = 10;
const HEADER_ROWS: u16 = 3;
const FOOTER_ROWS: u16 = 2;
const ROW_HOT_ACCENT: Color = Color::Rgb {
    r: 46,
    g: 102,
    b: 161,
};
const HEADER_ACCENT: Color = Color::Rgb {
    r: 130,
    g: 210,
    b: 255,
};
const HINT_ACCENT: Color = Color::Rgb {
    r: 140,
    g: 220,
    b: 195,
};

pub fn pick_thread(threads: &[Thread]) -> anyhow::Result<Option<String>> {
    if threads.is_empty() {
        println!("No saved threads found.");
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let start_row = reserve_picker_space(&mut stdout, picker_height(threads.len()))?;
    let raw = RawModeGuard::enter()?;
    execute!(stdout, Hide)?;
    let current_dir = std::env::current_dir()
        .ok()
        .and_then(|dir| dir.to_str().map(str::to_string));
    let mut only_current_directory = false;

    let mut query = String::new();
    let mut selected = 0usize;
    let mut matches = filtered_threads(
        threads,
        &query,
        only_current_directory,
        current_dir.as_deref(),
    );
    render(
        &mut stdout,
        start_row,
        &query,
        threads,
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
                break Some(threads[index].id.clone());
            }
            Event::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) => {
                selected = selected.saturating_sub(1);
            }
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) if selected + 1 < matches.len() => {
                selected += 1;
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('f'),
                modifiers,
                ..
            }) if modifiers == KeyModifiers::CONTROL => {
                only_current_directory = !only_current_directory;
                matches = filtered_threads(
                    threads,
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
                matches = filtered_threads(
                    threads,
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
                matches = filtered_threads(
                    threads,
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
            threads,
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
    if let Some(thread_id) = picked.as_deref()
        && let Some(thread) = threads.iter().find(|thread| thread.id == thread_id)
    {
        println!(
            "Resuming {} ({})",
            thread_title(thread),
            short_id(&thread.id)
        );
    }
    Ok(picked)
}

fn render(
    stdout: &mut io::Stdout,
    start_row: u16,
    query: &str,
    threads: &[Thread],
    matches: &[usize],
    selected: usize,
    only_current_directory: bool,
) -> anyhow::Result<()> {
    let width = render_width();
    let visible = visible_thread_count(matches.len(), start_row);
    let scroll_start = thread_window_start(selected, matches.len(), visible);
    execute!(
        stdout,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown)
    )?;
    render_header_line(
        stdout,
        start_row,
        &format!(
            "Search saved threads ({}): {query}",
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
            "No matching threads.",
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
        let thread = &threads[index];
        let is_selected = scroll_start + row == selected;
        let mut line = thread_line(thread, width);
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
        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
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
        SetForegroundColor(HEADER_ACCENT)
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
        SetForegroundColor(HINT_ACCENT),
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

fn visible_thread_count(match_count: usize, start_row: u16) -> usize {
    let height = terminal::size().map(|(_, height)| height).unwrap_or(24);
    let available_rows = height
        .saturating_sub(start_row)
        .saturating_sub(HEADER_ROWS)
        .saturating_sub(2);
    let visible_by_height = usize::from(available_rows.max(1));
    match_count.min(MAX_VISIBLE_THREADS).min(visible_by_height)
}

fn picker_height(match_count: usize) -> u16 {
    HEADER_ROWS
        .saturating_add(match_count.clamp(1, MAX_VISIBLE_THREADS) as u16)
        .saturating_add(FOOTER_ROWS)
}

fn reserve_picker_space(stdout: &mut io::Stdout, desired_height: u16) -> anyhow::Result<u16> {
    let (_, start_row) = crossterm::cursor::position()?;
    let (_, terminal_height) = terminal::size().unwrap_or((80, 24));
    let desired_height = desired_height.max(1).min(terminal_height.max(1));
    if rows_available_from(start_row, terminal_height) >= desired_height {
        return Ok(start_row);
    }

    for _ in 0..desired_height {
        execute!(stdout, Print("\r\n"))?;
    }
    stdout.flush()?;
    let (_, bottom_row) = crossterm::cursor::position()?;
    Ok(reserved_start_row(bottom_row, desired_height))
}

fn rows_available_from(row: u16, terminal_height: u16) -> u16 {
    terminal_height.saturating_sub(row)
}

fn reserved_start_row(bottom_row: u16, desired_height: u16) -> u16 {
    bottom_row.saturating_sub(desired_height.saturating_sub(1))
}

fn clamp_selection(selected: usize, match_count: usize) -> usize {
    if match_count == 0 {
        0
    } else {
        selected.min(match_count.saturating_sub(1))
    }
}

fn thread_window_start(selected: usize, match_count: usize, visible: usize) -> usize {
    if match_count <= visible {
        return 0;
    }

    let half_window = visible / 2;
    let max_start = match_count.saturating_sub(visible);
    selected.saturating_sub(half_window).min(max_start)
}

fn filtered_threads(
    threads: &[Thread],
    query: &str,
    only_current_directory: bool,
    current_dir: Option<&str>,
) -> Vec<usize> {
    let current_path = current_dir.map(|dir| normalize_path_for_filter(Path::new(dir)));
    let query = query.trim().to_ascii_lowercase();
    let mut matches: Vec<_> = threads
        .iter()
        .enumerate()
        .filter_map(|(index, thread)| {
            if only_current_directory
                && !thread_in_current_directory(Some(thread.cwd.as_str()), current_path.as_deref())
            {
                return None;
            }
            if query.is_empty() || searchable_text(thread).contains(&query) {
                Some(index)
            } else {
                None
            }
        })
        .collect();

    matches.sort_by(|left, right| threads[*right].updated_at.cmp(&threads[*left].updated_at));
    matches
}

fn thread_in_current_directory(thread_workspace: Option<&str>, current_dir: Option<&Path>) -> bool {
    let Some(current_dir) = current_dir else {
        return false;
    };
    let Some(thread_workspace) = thread_workspace else {
        return false;
    };
    let normalized_thread_workspace = normalize_path_for_filter(Path::new(thread_workspace));
    normalized_thread_workspace.starts_with(current_dir)
}

fn normalize_path_for_filter(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn searchable_text(thread: &Thread) -> String {
    [
        thread_title(thread),
        thread.cwd.clone(),
        thread.model_provider.clone(),
        thread.model.clone(),
        thread.id.clone(),
    ]
    .join(" ")
    .to_ascii_lowercase()
}

fn thread_title(thread: &Thread) -> String {
    thread
        .name
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| (!thread.preview.trim().is_empty()).then(|| thread.preview.clone()))
        .unwrap_or_else(|| format!("Thread {}", short_id(&thread.id)))
}

fn thread_line(thread: &Thread, width: usize) -> String {
    let workspace = compacted_workspace(Some(thread.cwd.as_str()));
    let model = thread_model_label(thread);
    let date = human_time(thread.updated_at);
    let message_count = thread_message_count(thread);
    let title = thread_title(thread);
    let short_id = short_id(&thread.id);

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
    let model = truncate(&model, model_width);
    let right = truncate(&right, right_width.max(1));

    let line = format!(
        "{}  {}  |  {}  |  {}  |  {}",
        date, title, workspace, model, right,
    );
    fit_line(&line, width)
}

fn thread_model_label(thread: &Thread) -> String {
    if thread.model.trim().is_empty() {
        thread.model_provider.clone()
    } else if thread.model_provider.trim().is_empty() {
        thread.model.clone()
    } else {
        format!("{}/{}", thread.model_provider, thread.model)
    }
}

fn thread_message_count(thread: &Thread) -> usize {
    if let Some(message_count) = thread.message_count {
        return message_count as usize;
    }
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| matches!(item, Item::UserMessage { .. } | Item::AgentMessage { .. }))
        .count()
}

fn human_time(ts: i64) -> String {
    let ts =
        time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
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
    use roder_protocol::{Item, ThreadStatus, Turn};
    use time::{Duration, OffsetDateTime};

    use super::*;

    #[test]
    fn filters_threads_by_title_workspace_and_id() {
        let threads = vec![
            thread(
                "thread-alpha",
                Some("Fix resume menu"),
                Some("/Users/pz/w/gode"),
            ),
            thread("thread-beta", Some("Other work"), Some("/tmp/example")),
        ];

        let no_query = filtered_threads(&threads, "", false, None);
        assert_eq!(no_query.len(), 2);
        assert_eq!(filtered_threads(&threads, "gode", false, None), vec![0]);
        assert_eq!(filtered_threads(&threads, "beta", false, None), vec![1]);
        assert_eq!(filtered_threads(&threads, "resume", false, None), vec![0]);
    }

    #[test]
    fn filters_only_current_directory() {
        let threads = vec![
            thread(
                "thread-alpha",
                Some("Fix resume menu"),
                Some("/Users/pz/w/gode"),
            ),
            thread("thread-beta", Some("Other work"), Some("/tmp/example")),
        ];

        let current = Some("/Users/pz/w/gode");
        let all = filtered_threads(&threads, "", false, current);
        let local = filtered_threads(&threads, "", true, current);

        assert_eq!(all.len(), 2);
        assert_eq!(local, vec![0]);
    }

    #[test]
    fn thread_line_includes_directory_model_count_and_short_id() {
        let thread = thread(
            "123456789abcdef",
            Some("Fix resume menu"),
            Some("/Users/pz/w/gode"),
        );

        let detail = thread_line(&thread, 220);

        assert!(detail.contains("gode"));
        assert!(detail.contains("mock"));
        assert!(detail.contains("2 msg"));
        assert!(detail.contains("12345678"));
    }

    #[test]
    fn picker_height_caps_to_visible_rows_plus_chrome() {
        assert_eq!(picker_height(0), HEADER_ROWS + 1 + FOOTER_ROWS);
        assert_eq!(picker_height(3), HEADER_ROWS + 3 + FOOTER_ROWS);
        assert_eq!(
            picker_height(MAX_VISIBLE_THREADS + 20),
            HEADER_ROWS + MAX_VISIBLE_THREADS as u16 + FOOTER_ROWS
        );
    }

    #[test]
    fn reserved_start_row_moves_origin_above_reserved_space() {
        assert_eq!(rows_available_from(20, 24), 4);
        assert_eq!(reserved_start_row(23, 15), 9);
    }

    #[test]
    fn fit_line_stays_within_terminal_width() {
        let line = fit_line("abcdefghijklmnopqrstuvwxyz", 10);

        assert_eq!(line.chars().count(), 10);
        assert_eq!(line, "abcdefg...");
    }

    fn thread(id: &str, title: Option<&str>, workspace: Option<&str>) -> Thread {
        thread_with_updated_at(
            id,
            title,
            workspace,
            OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
        )
    }

    #[test]
    fn sorts_filtered_threads_by_updated_at_desc() {
        let older = thread_with_updated_at(
            "thread-old",
            Some("Older thread"),
            Some(std::env::temp_dir().to_string_lossy().as_ref()),
            OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
        );
        let newer = thread_with_updated_at(
            "thread-new",
            Some("Newer thread"),
            Some(std::env::temp_dir().to_string_lossy().as_ref()),
            OffsetDateTime::UNIX_EPOCH + Duration::seconds(10),
        );
        let threads = vec![older, newer];

        let results = filtered_threads(&threads, "", false, None);
        assert_eq!(results, vec![1, 0]);
    }

    fn thread_with_updated_at(
        id: &str,
        title: Option<&str>,
        workspace: Option<&str>,
        updated_at: OffsetDateTime,
    ) -> Thread {
        Thread {
            id: id.to_string(),
            preview: title.unwrap_or("Untitled thread").to_string(),
            model_provider: "mock".to_string(),
            model: "mock".to_string(),
            created_at: OffsetDateTime::UNIX_EPOCH.unix_timestamp(),
            updated_at: updated_at.unix_timestamp(),
            status: ThreadStatus {
                kind: "idle".to_string(),
                active_turn_id: None,
                active_flags: Vec::new(),
            },
            cwd: workspace.unwrap_or("/tmp").to_string(),
            workspace_id: None,
            root_id: None,
            name: title.map(str::to_string),
            message_count: None,
            turns: Some(vec![Turn {
                id: "turn-a".to_string(),
                items: vec![user_message("hi"), agent_message("hello")],
                items_view: "default".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                usage: None,
            }]),
            usage: None,
        }
    }

    fn user_message(text: &str) -> Item {
        Item::UserMessage {
            id: "userMessage-id".to_string(),
            text: text.to_string(),
            images: Vec::new(),
            status: None,
        }
    }

    fn agent_message(text: &str) -> Item {
        Item::AgentMessage {
            id: "agentMessage-id".to_string(),
            text: text.to_string(),
            phase: None,
            status: None,
        }
    }
}
