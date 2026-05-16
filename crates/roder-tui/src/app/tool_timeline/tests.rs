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
    assert!(row.starts_with("  ◆ "));
    assert!(!row.starts_with("│"));
    assert!(!row.starts_with("  │"));
}

#[test]
fn user_prompts_render_as_full_width_bands() {
    let mut timeline = TimelineState::default();
    timeline.push_user("what does this repo do?");

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 64, 10));
    let row = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("what does this repo do?"))
        })
        .expect("user prompt should be rendered");
    let text = row
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.starts_with("  ❯ what does this repo do?"));
    assert_eq!(text.chars().count(), 64);
    assert!(
        row.spans
            .iter()
            .all(|span| span.style.bg == Some(Theme::for_dark_background(true).user_bg))
    );
}

#[test]
fn consecutive_tool_rows_stay_compact() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("list_files", ""),
    );
    timeline.record_tool_requested(
        "call_2".to_string(),
        ToolTimelineEntry::new("grep", r#"{"query":"repo","path":"."}"#),
    );

    let lines = rendered_lines(&mut timeline);
    let list_row = lines
        .iter()
        .position(|line| line.contains("List Files"))
        .expect("list row should be rendered");
    let grep_row = lines
        .iter()
        .position(|line| line.contains("Grep path: . query: repo"))
        .expect("grep row should be rendered");

    assert_eq!(grep_row, list_row + 1);
}

#[test]
fn timeline_collapses_older_tools_behind_more_tools_row() {
    let mut timeline = TimelineState::default();
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"file-{index}.rs"}}"#)),
        );
    }

    let lines = rendered_lines(&mut timeline);

    assert!(lines.iter().any(|line| line.contains("› 2 more")));
    assert!(!lines.iter().any(|line| line.contains("file-0.rs")));
    assert!(!lines.iter().any(|line| line.contains("file-1.rs")));
    assert!(lines.iter().any(|line| line.contains("file-2.rs")));
    assert!(lines.iter().any(|line| line.contains("file-7.rs")));
}

#[test]
fn clicking_more_tools_row_reveals_all_tools() {
    let mut timeline = TimelineState::default();
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"file-{index}.rs"}}"#)),
        );
    }
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 20));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    let lines = rendered_lines(&mut timeline);
    assert!(!lines.iter().any(|line| line.contains("› 2 more")));
    assert!(lines.iter().any(|line| line.contains("file-0.rs")));
    assert!(lines.iter().any(|line| line.contains("file-7.rs")));
}

#[test]
fn clicking_assistant_response_does_not_select_or_highlight_it() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("final answer", None);
    let theme = Theme::for_dark_background(true);
    let area = Rect::new(0, 2, 100, 10);
    timeline.render(theme, area);

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 2,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(!timeline.handle_mouse(click));
    assert_eq!(timeline.selected, None);

    let render = timeline.render(theme, area);
    let response = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("final answer"))
        })
        .expect("assistant response should render");
    assert!(
        response
            .spans
            .iter()
            .all(|span| span.style.bg != Some(theme.selection_bg))
    );
}

#[test]
fn clicking_reasoning_response_does_not_select_or_highlight_it() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("working through it");
    let theme = Theme::for_dark_background(true);
    let area = Rect::new(0, 2, 100, 10);
    timeline.render(theme, area);

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 2,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(!timeline.handle_mouse(click));
    assert_eq!(timeline.selected, None);

    let render = timeline.render(theme, area);
    let response = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("working through it"))
        })
        .expect("reasoning response should render");
    assert!(
        response
            .spans
            .iter()
            .all(|span| span.style.bg != Some(theme.selection_bg))
    );
}

#[test]
fn more_tools_row_matches_collapsed_tool_header_style() {
    let mut timeline = TimelineState::default();
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"file-{index}.rs"}}"#)),
        );
    }

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 80, 20));
    let row = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("› 2 more"))
        })
        .expect("more tools row should render");
    let text = row
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(text.trim_end(), "  › 2 more");
    assert_eq!(text.chars().count(), 80);
    assert!(
        row.spans
            .iter()
            .all(|span| span.style.bg == Some(Theme::for_dark_background(true).user_bg))
    );
}

#[test]
fn running_tool_marker_fades_with_animation_frame() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("grep", r#"{"query":"repo","path":"."}"#),
    );

    let first = timeline.render_with_frame(
        Theme::for_dark_background(true),
        Rect::new(0, 0, 100, 10),
        0,
    );
    let second = timeline.render_with_frame(
        Theme::for_dark_background(true),
        Rect::new(0, 0, 100, 10),
        3,
    );
    let first_marker = first
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.contains('◆'))
        .expect("running marker should render");
    let second_marker = second
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.contains('◆'))
        .expect("running marker should render");

    assert_ne!(first_marker.style.fg, second_marker.style.fg);
    assert_eq!(first_marker.style.bg, None);
    assert_eq!(second_marker.style.bg, None);
}

#[test]
fn user_prompt_continuation_lines_keep_prompt_band_width() {
    let mut timeline = TimelineState::default();
    timeline.push_user("first\nsecond");

    let rows = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 40, 10))
        .text
        .lines
        .iter()
        .take(2)
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert_eq!(rows[0].trim_end(), "  ❯ first");
    assert_eq!(rows[1].trim_end(), "    second");
    assert!(rows.iter().all(|row| row.chars().count() == 40));
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

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Edited src/lib.rs (+1 -1)"))
    );
    assert!(lines.iter().any(|line| line == "     ⋮"));
    assert!(lines.iter().any(|line| line == "    1 -old"));
    assert!(lines.iter().any(|line| line == "    1 +new"));

    let theme = Theme::for_dark_background(true);
    let removed = render
        .text
        .lines
        .iter()
        .find(|line| line.spans.iter().any(|span| span.content.as_ref() == "old"))
        .expect("removed diff line should be rendered");
    assert!(
        removed
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "old"
                && span.style.bg == Some(theme.diff_removed_bg))
    );
    let added = render
        .text
        .lines
        .iter()
        .find(|line| line.spans.iter().any(|span| span.content.as_ref() == "new"))
        .expect("added diff line should be rendered");
    assert!(
        added.spans.iter().any(
            |span| span.content.as_ref() == "new" && span.style.bg == Some(theme.diff_added_bg)
        )
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
            .any(|line| line.contains("Edited src/lib.rs (+1 -1)"))
    );
    assert!(lines.iter().any(|line| line == "    1 -old line"));
    assert!(lines.iter().any(|line| line == "    1 +new line"));
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
            .any(|line| line.contains("Edited src/lib.rs (+2 -3)"))
    );
    assert!(lines.iter().any(|line| line == "    1 -alpha"));
    assert!(lines.iter().any(|line| line == "    1 +beta"));
    assert!(lines.iter().any(|line| line == "    2 -one"));
    assert!(lines.iter().any(|line| line == "    3 -two"));
    assert!(lines.iter().any(|line| line == "    2 +three"));
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
            .any(|line| line.contains("Wrote src/lib.rs (+2 -0)"))
    );
    assert!(lines.iter().any(|line| line == "    1 +first"));
    assert!(lines.iter().any(|line| line == "    2 +second"));
}

#[test]
fn commentary_phase_deltas_render_as_plain_assistant_text() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("I will inspect first.", Some("commentary".to_string()));
    timeline.push_assistant_delta(" Then run tests.", Some("commentary".to_string()));
    timeline.push_assistant_delta("Done.", Some("final_answer".to_string()));

    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line == "I will inspect first. Then run tests.")
    );
    assert!(!lines.iter().any(|line| line.contains("Commentary:")));
    assert!(lines.iter().any(|line| line == "Done."));
}

#[test]
fn tools_render_after_commentary_phase_messages() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("I will inspect first.", Some("commentary".to_string()));
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );

    let lines = rendered_lines(&mut timeline);
    let commentary_row = lines
        .iter()
        .position(|line| line == "I will inspect first.")
        .expect("commentary row should render");
    let tool_row = lines
        .iter()
        .position(|line| line.contains("Read File path: README.md"))
        .expect("tool row should render");

    assert_eq!(tool_row, commentary_row + 1);
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
        reasoning_tokens: Some(512),
        session_tokens: 100_200,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed in"))
        .expect("turn completion row should be rendered");

    assert!(row.starts_with("  Turn completed in 1.2 sec."));
    assert!(
        row.trim_end()
            .ends_with("↑ 46K  ↓ 1.2K  thinking 512  session 100K tokens")
    );
}

#[test]
fn turn_completed_duration_summary_uses_human_units() {
    let mut timeline = TimelineState::default();
    timeline.push_turn_completed(TurnCompletedSummary {
        elapsed: Duration::from_secs(125),
        input_tokens: 400,
        output_tokens: 20,
        reasoning_tokens: None,
        session_tokens: 420,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed in"))
        .expect("turn completion row should be rendered");

    assert_eq!(
        row.trim_end(),
        "  Turn completed in 2 min 5 sec.  ↑ 400  ↓ 20  session 420 tokens"
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
fn mouse_wheel_scrolls_fast_and_reverses_direction_immediately() {
    let mut timeline = TimelineState::default();
    for index in 0..40 {
        timeline.push_system(format!("event {index}"));
    }
    let area = Rect::new(0, 0, 100, 5);
    let at_bottom = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    let wheel_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 2,
        row: 4,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(wheel_up));
    let after_up = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    assert_eq!(
        usize::from(at_bottom - after_up),
        MOUSE_SCROLL_ROWS as usize
    );

    let wheel_down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 4,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(wheel_down));
    let after_down = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    assert_eq!(after_down, at_bottom);
}

#[test]
fn page_keys_use_viewport_sized_scroll_steps() {
    let mut timeline = TimelineState::default();
    for index in 0..80 {
        timeline.push_system(format!("event {index}"));
    }
    let area = Rect::new(0, 0, 100, 30);
    let at_bottom = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    timeline.focus_latest();
    assert!(timeline.handle_key(key(KeyCode::PageUp)));
    let after_page_up = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    assert_eq!(at_bottom - after_page_up, area.height - 1);
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

#[test]
fn clicking_shell_tool_requests_detail_modal() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"make test"}"#),
    );
    timeline.record_tool_completed(
        "call_1",
        false,
        Some("Exit code: 0\nWall time: 1.000 seconds\nOutput:\nok".to_string()),
    );
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 3, 100, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 3,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    let detail = timeline
        .take_requested_detail()
        .expect("shell tool click should request detail modal");
    assert_eq!(detail.command.as_deref(), Some("make test"));
    assert!(detail.output.as_deref().unwrap().contains("Output:\nok"));
}

#[test]
fn clicking_non_shell_tool_does_not_request_detail_modal() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("contents".to_string()));
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 3, 100, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 3,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    assert!(timeline.take_requested_detail().is_none());
}

#[test]
fn mouse_hit_testing_accounts_for_wrapped_tool_rows() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("grep", r#"{"query":"123456789012345","path":"."}"#),
    );
    timeline.record_tool_requested(
        "call_2".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 10, 30, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 12,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    assert_eq!(timeline.selected, Some(1));
}
