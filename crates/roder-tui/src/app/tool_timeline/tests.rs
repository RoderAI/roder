use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::Duration;

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

fn visible_lines(timeline: &mut TimelineState, height: u16) -> Vec<String> {
    let render = timeline.render(
        Theme::for_dark_background(true),
        Rect::new(0, 0, 100, height),
    );
    let scroll = usize::from(render.scroll);
    render
        .text
        .lines
        .iter()
        .skip(scroll)
        .take(usize::from(height))
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
fn apply_patch_renders_streaming_inline_diff() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("apply_patch", ""),
    );
    timeline.record_tool_delta(
        "call_1",
        "{\"patch\":\"*** Begin Patch\\n*** Update File: src/lib.rs\\n@@\\n-old\\n+new",
    );

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 20));
    let lines = render
        .text
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(lines.iter().any(|line| line.contains("Apply Patch")));
    assert!(
        lines
            .iter()
            .any(|line| line == "  *** Update File: src/lib.rs")
    );
    assert!(lines.iter().any(|line| line == "  -old"));
    assert!(lines.iter().any(|line| line == "  +new"));

    let removed = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "-old")
        })
        .expect("removed diff line should be rendered");
    assert!(
        removed
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "-old"
                && span.style.add_modifier.contains(Modifier::BOLD))
    );
}

#[test]
fn edit_tool_renders_streaming_inline_diff() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested("call_1".to_string(), ToolTimelineEntry::new("edit", ""));
    timeline.record_tool_delta(
        "call_1",
        "{\"path\":\"src/lib.rs\",\"old_string\":\"old line\",\"new_string\":\"new line",
    );

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line == "  *** Edit File: src/lib.rs")
    );
    assert!(lines.iter().any(|line| line == "  -old line"));
    assert!(lines.iter().any(|line| line == "  +new line"));
}

#[test]
fn multi_edit_tool_renders_inline_diff() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new(
            "multi_edit",
            serde_json::json!({
                "path": "src/lib.rs",
                "edits": [
                    { "old_string": "alpha", "new_string": "beta" },
                    { "old_string": "one\ntwo", "new_string": "three" }
                ]
            })
            .to_string(),
        ),
    );

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line == "  *** Edit File: src/lib.rs")
    );
    assert!(lines.iter().any(|line| line == "  -alpha"));
    assert!(lines.iter().any(|line| line == "  +beta"));
    assert!(lines.iter().any(|line| line == "  -one"));
    assert!(lines.iter().any(|line| line == "  -two"));
    assert!(lines.iter().any(|line| line == "  +three"));
}

#[test]
fn write_file_tool_renders_inline_diff() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new(
            "write_file",
            serde_json::json!({
                "path": "src/lib.rs",
                "content": "first\nsecond\n"
            })
            .to_string(),
        ),
    );

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line == "  *** Write File: src/lib.rs")
    );
    assert!(lines.iter().any(|line| line == "  +first"));
    assert!(lines.iter().any(|line| line == "  +second"));
}

#[test]
fn commentary_phase_deltas_render_with_phase_label() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("I will inspect first.", Some("commentary".to_string()));
    timeline.push_assistant_delta(" Then run tests.", Some("commentary".to_string()));
    timeline.push_assistant_delta("Done.", Some("final_answer".to_string()));

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("commentary I will inspect first. Then run tests."))
    );
    assert!(lines.iter().any(|line| line == "Done."));
}

#[test]
fn final_assistant_messages_render_markdown() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta(
        "# Result\n\n- **Fast** path with `code`\n1. _Slow_ path\n```rust\nlet x = 1;\n```",
        Some("final_answer".to_string()),
    );

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 20));
    let lines = render
        .text
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(lines.iter().any(|line| line == "Result"));
    assert!(lines.iter().any(|line| line.is_empty()));
    assert!(lines.iter().any(|line| line == "- Fast path with code"));
    assert!(lines.iter().any(|line| line == "1. Slow path"));
    assert!(lines.iter().any(|line| line == "    let x = 1;"));
    assert!(!lines.iter().any(|line| line.contains("**")));
    assert!(!lines.iter().any(|line| line.contains('`')));

    let heading = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "Result")
        })
        .expect("heading should be rendered");
    assert!(
        heading
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "Result"
                && span.style.add_modifier.contains(Modifier::BOLD))
    );
}

#[test]
fn reasoning_deltas_render_live_as_thinking() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("The user is asking for ");
    timeline.push_reasoning_delta("visible thinking tokens.");
    timeline.push_assistant_delta("Done.", Some("final_answer".to_string()));

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line == "Thinking: The user is asking for visible thinking tokens.")
    );
    assert!(lines.iter().any(|line| line == "Done."));
}

#[test]
fn turn_completed_renders_right_aligned_usage_summary() {
    let mut timeline = TimelineState::default();
    timeline.push_turn_completed(TurnCompletedSummary {
        elapsed: Duration::from_millis(1234),
        input_tokens: 46_694,
        output_tokens: 1_240,
        session_tokens: 100_200,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed."))
        .expect("turn completion row should be rendered");

    assert!(row.starts_with("    Turn completed."));
    assert!(row.ends_with("1.2 sec  in 46K  out 1.2K  session 100K tokens"));
}

#[test]
fn turn_completed_duration_summary_uses_human_units() {
    let mut timeline = TimelineState::default();
    timeline.push_turn_completed(TurnCompletedSummary {
        elapsed: Duration::from_secs(125),
        input_tokens: 400,
        output_tokens: 20,
        session_tokens: 420,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed."))
        .expect("turn completion row should be rendered");

    assert!(row.ends_with("2 min 5 sec  in 400  out 20  session 420 tokens"));
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
fn timeline_auto_follows_streaming_deltas() {
    let mut timeline = TimelineState::default();
    for index in 0..8 {
        timeline.push_system(format!("event {index}"));
    }

    let first = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));
    let first_scroll = first.scroll;

    timeline.push_reasoning_delta("line 1");
    timeline.push_reasoning_delta("\nline 2\nline 3\nline 4");
    let second = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));

    assert!(second.scroll > first_scroll);
    let visible = visible_lines(&mut timeline, 5);
    assert!(visible.iter().any(|line| line == "    line 4"));
    assert_eq!(visible[2], "");
    assert_eq!(visible[3], "");
    assert_eq!(visible[4], "");
}

#[test]
fn timeline_manual_scroll_disables_auto_follow_until_end() {
    let mut timeline = TimelineState::default();
    for index in 0..12 {
        timeline.push_system(format!("event {index}"));
    }
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));

    timeline.focus_latest();
    assert!(timeline.handle_key(key(KeyCode::PageUp)));
    let scrolled = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));
    timeline.push_system("event after manual scroll");
    let after_push = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));
    assert_eq!(after_push.scroll, scrolled.scroll);

    assert!(timeline.handle_key(key(KeyCode::End)));
    let followed = visible_lines(&mut timeline, 5);
    assert!(
        followed
            .iter()
            .any(|line| line.contains("event after manual scroll"))
    );
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
