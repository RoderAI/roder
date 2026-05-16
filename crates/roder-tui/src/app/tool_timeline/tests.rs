use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::super::Theme;
use super::*;

fn rendered_lines(timeline: &mut TimelineState) -> Vec<String> {
    timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 20))
        .text
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn tool_entry_formats_title_and_arguments() {
    let entry = ToolTimelineEntry::new(
        "grep",
        r##"{"pattern":"^#|name =|description","path":"README.md"}"##,
    );

    assert_eq!(
        entry.label(),
        r#"Grep path: README.md pattern: "^#|name =|description""#
    );
}

#[test]
fn timeline_updates_tool_rows_in_place() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("contents".to_string()));

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Read File path: README.md"))
    );
    assert_eq!(
        timeline
            .items
            .iter()
            .filter(|item| matches!(item.kind, TimelineItemKind::Tool(_)))
            .count(),
        1
    );
}

#[test]
fn tool_rows_do_not_render_a_left_gutter() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("grep", r#"{"query":"timeline","path":"."}"#),
    );

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Grep path: . query: timeline"))
        .expect("tool row should be rendered");
    assert!(row.starts_with("◆ "));
    assert!(!row.starts_with("│"));
    assert!(!row.starts_with("  │"));
}

#[test]
fn keyboard_navigation_expands_selected_tool_output() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("grep", r#"{"query":"needle","path":"src"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("src/lib.rs:1:needle".to_string()));

    timeline.focus_latest();
    assert!(timeline.handle_key(key(KeyCode::Enter)));

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("src/lib.rs:1:needle"))
    );
}

#[test]
fn space_is_not_consumed_by_timeline_focus() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"src/main.rs"}"#),
    );

    timeline.focus_latest();
    assert!(!timeline.handle_key(key(KeyCode::Char(' '))));
}

#[test]
fn keyboard_and_mouse_can_scroll_timeline() {
    let mut timeline = TimelineState::default();
    for index in 0..20 {
        timeline.push_system(format!("event {index}"));
    }

    timeline.focus_latest();
    assert!(timeline.handle_key(key(KeyCode::PageUp)));
    let first_scroll = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5))
        .scroll;

    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 4,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(wheel));
    let second_scroll = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5))
        .scroll;
    assert!(second_scroll >= first_scroll);
}

#[test]
fn mouse_click_selects_and_second_click_expands_tool() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("glob", r#"{"pattern":"**/*.rs"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("src/main.rs".to_string()));
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 4, 100, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 4,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(click));
    assert!(timeline.handle_mouse(click));

    let lines = rendered_lines(&mut timeline);
    assert!(lines.iter().any(|line| line.contains("src/main.rs")));
}
