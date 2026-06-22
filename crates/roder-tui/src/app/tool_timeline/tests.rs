use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier},
};
use roder_api::extension_state::ExtensionStoreScope;
use roder_api::interactive::RegionKind;
use std::time::{Duration, Instant};

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
    let scroll = usize::from(render.text_scroll);
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
        r#"Grep: README.md pattern: "^#|name =|description""#
    );
}

#[test]
fn hosted_web_search_tool_entry_shows_query() {
    let entry = ToolTimelineEntry::new(
        "web_search",
        r#"{"action":"search","query":"pandelis zembashis"}"#,
    );

    assert_eq!(
        entry.label(),
        "Web Search: query: \"pandelis zembashis\" action: search"
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

    let lines = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 120))
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
            .any(|line| line.contains("Read File") && line.contains("README.md"))
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
        .find(|line| line.contains("Grep") && line.contains("timeline"))
        .expect("tool row should be rendered");
    assert!(!row.starts_with("│"));
    assert!(!row.starts_with("  │"));
}

#[test]
fn user_prompts_render_as_full_width_widget() {
    let mut timeline = TimelineState::default();
    timeline.push_user("what does this repo do?");

    let theme = Theme::for_dark_background(true);
    let render = timeline.render(theme, Rect::new(0, 0, 64, 10));
    let rows = render
        .text
        .lines
        .iter()
        .filter(|line| {
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(theme.user_message_bg))
        })
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 3, "top padding, content row, bottom padding");
    for row in &rows {
        let text = row
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text.chars().count(), 64);
        assert!(text.starts_with("▌"));
        assert!(row.spans.iter().all(|span| {
            span.style.bg == Some(theme.user_message_bg)
                || span.style.bg == Some(theme.selection_bg)
        }));
        assert_eq!(row.spans[0].style.fg, Some(theme.accent));
    }

    let text = rows[1]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.starts_with("▌ ❯ what does this repo do?"));
}

#[test]
fn entrypoint_hints_render_as_compact_timeline_rows() {
    let mut timeline = TimelineState::default();
    timeline.push_system(
        "Likely entry points:\n\
1. docs/roder-plan-review-hunk-tracker.md - path matches `view`\n\
2. crates/roder-tui/src/app/tool_timeline/preview.rs - path matches `timeline`\n\
3. crates/roder-core/src/plan_review.rs - path matches `view`",
    );

    let lines = rendered_lines(&mut timeline);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Likely entry points (3)"))
    );
    assert!(lines.iter().any(|line| {
        line.contains("1.")
            && line.contains("docs/roder-plan-review-hunk-tracker.md")
            && line.contains("path matches `view`")
    }));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Likely entry points:"))
    );
}

#[test]
fn entrypoint_hints_in_user_blocks_skip_prompt_prefix() {
    let mut timeline = TimelineState::default();
    timeline.push_user(
        "Likely entry points:\n\
1. crates/roder-tui/src/app/tool_timeline/preview.rs - path matches `timeline`",
    );

    let lines = rendered_lines(&mut timeline);
    let header = lines
        .iter()
        .find(|line| line.contains("Likely entry points"))
        .expect("entrypoint header should render");

    assert!(header.contains("⌖"));
    assert!(!header.contains("❯"));
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
        .position(|line| line.contains("Grep") && line.contains("repo"))
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
fn timeline_collapses_tools_between_assistant_message_groups() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("First commentary.", Some("commentary".to_string()));
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("first_call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"first-{index}.rs"}}"#)),
        );
    }
    timeline.push_assistant_delta("Second commentary.", Some("commentary".to_string()));
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("second_call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"second-{index}.rs"}}"#)),
        );
    }

    let lines = rendered_lines(&mut timeline);
    let first_commentary_row = lines
        .iter()
        .position(|line| line == "First commentary.")
        .expect("first commentary should render");
    let second_commentary_row = lines
        .iter()
        .position(|line| line == "Second commentary.")
        .expect("second commentary should render");
    let more_rows = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains("› 2 more").then_some(index))
        .collect::<Vec<_>>();

    assert_eq!(more_rows.len(), 2);
    assert!(more_rows[0] > first_commentary_row);
    assert!(more_rows[0] < second_commentary_row);
    assert!(more_rows[1] > second_commentary_row);
    assert!(!lines.iter().any(|line| line.contains("first-0.rs")));
    assert!(!lines.iter().any(|line| line.contains("first-1.rs")));
    assert!(lines.iter().any(|line| line.contains("first-2.rs")));
    assert!(!lines.iter().any(|line| line.contains("second-0.rs")));
    assert!(!lines.iter().any(|line| line.contains("second-1.rs")));
    assert!(lines.iter().any(|line| line.contains("second-2.rs")));
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
    assert!(row.spans.iter().all(|span| span.style.bg.is_none()));
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
        .find(|span| span.content.contains('↻'))
        .expect("running marker should render");
    let second_marker = second
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.contains('↻'))
        .expect("running marker should render");

    assert_ne!(first_marker.style.fg, second_marker.style.fg);
    assert_eq!(first_marker.style.bg, None);
    assert_eq!(second_marker.style.bg, None);
}

#[test]
fn user_prompt_continuation_lines_keep_prompt_width() {
    let mut timeline = TimelineState::default();
    timeline.push_user("first\nsecond");

    // Rows: [0] blank top, [1] "▌ ❯ first", [2] "▌   second", [3] blank bottom.
    let rows = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 40, 10))
        .text
        .lines
        .iter()
        .skip(1) // skip blank top padding row
        .take(2)
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert_eq!(rows[0].trim_end(), "▌ ❯ first");
    assert_eq!(rows[1].trim_end(), "▌   second");
    assert!(rows.iter().all(|row| row.chars().count() == 40));
}

#[test]
fn user_prompt_long_line_wraps_instead_of_truncating() {
    let mut timeline = TimelineState::default();
    // Message that exceeds 40-char terminal width.
    // " ❯ " prefix = 3 chars, rail "▌" = 1 char → text fits in 36 chars.
    timeline.push_user("the quick brown fox jumps over the lazy dog and keeps running");

    let rows = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 40, 10))
        .text
        .lines
        .iter()
        .skip(1) // skip blank top padding row
        .take_while(|line| {
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(Theme::for_dark_background(true).user_message_bg))
        })
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    // Should produce multiple rows (wrapped), not a single truncated "..." row.
    assert!(rows.len() >= 2, "long message should wrap to multiple rows");
    assert!(
        rows.iter().all(|row| row.chars().count() == 40),
        "each wrapped row should fill the terminal width"
    );
    // Entire message content should be visible across wrapped rows, not cut off.
    let combined: String = rows.iter().map(|r| r.trim()).collect::<Vec<_>>().join(" ");
    assert!(
        !combined.contains("..."),
        "text should wrap, not truncate with ..."
    );
    assert!(
        combined.contains("lazy dog"),
        "wrapped content should contain full message"
    );
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
        "{\"patch\":\"*** Begin Patch\\n*** Update File: src/lib.rs\\n@@\\n-let old = call();\\n+let new = format!(\\\\\\\"value\\\\\\\");",
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
            .any(|line| line.contains("src/lib.rs") && line.contains("+1") && line.contains("-1"))
    );
    assert!(lines.iter().any(|line| line == "     ⋮"));
    assert!(lines.iter().any(|line| line == "    1 -let old = call();"));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("+let new = format!") && line.contains("value"))
    );

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
                && span.style.fg == Some(theme.text)
                && span.style.bg == Some(theme.diff_removed_bg))
    );
    assert!(
        removed
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "let"
                && span.style.fg == Some(theme.accent)
                && span.style.bg == Some(theme.diff_removed_bg))
    );
    let added = render
        .text
        .lines
        .iter()
        .find(|line| line.spans.iter().any(|span| span.content.as_ref() == "new"))
        .expect("added diff line should be rendered");
    assert!(added.spans.iter().any(|span| span.content.as_ref() == "new"
        && span.style.fg == Some(theme.text)
        && span.style.bg == Some(theme.diff_added_bg)));
    assert!(added.spans.iter().any(|span| {
        span.content.as_ref() == "let"
            && span.style.fg == Some(theme.accent)
            && span.style.bg == Some(theme.diff_added_bg)
    }));
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
            .any(|line| line.contains("src/lib.rs") && line.contains("+1") && line.contains("-1"))
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
            .any(|line| line.contains("src/lib.rs") && line.contains("+2") && line.contains("-3"))
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
            .any(|line| line.contains("src/lib.rs") && line.contains("+2") && line.contains("-0"))
    );
    assert!(lines.iter().any(|line| line == "    1 +first"));
    assert!(lines.iter().any(|line| line == "    2 +second"));
}

#[test]
fn large_write_file_preview_clips_above_and_keeps_newest_lines_visible() {
    let content = (1..=120)
        .map(|line| format!("line {line:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new(
            "write_file",
            serde_json::json!({
                "path": "src/large.rs",
                "content": content,
            })
            .to_string(),
        ),
    );

    let lines = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 120))
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

    assert!(lines.iter().any(|line| line.contains("src/large.rs")));
    assert!(lines.iter().any(|line| line.contains("clipped above")));
    assert!(!lines.iter().any(|line| line.contains("+line 001")));
    assert!(!lines.iter().any(|line| line.contains("+line 020")));
    assert!(lines.iter().any(|line| line.contains("+line 120")));
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
fn commentary_phase_deltas_render_with_white_text_on_dark_backgrounds() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta("I will inspect first.", Some("commentary".to_string()));
    timeline.push_assistant_delta("Done.", Some("final_answer".to_string()));

    let theme = Theme::for_dark_background(true);
    let render = timeline.render(theme, Rect::new(0, 0, 100, 20));
    let commentary_span = render
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.as_ref() == "I will inspect first.")
        .expect("commentary span should render");
    let final_span = render
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.as_ref() == "Done.")
        .expect("final answer span should render");

    assert_eq!(theme.commentary, Color::Indexed(15));
    assert_eq!(commentary_span.style.fg, Some(theme.commentary));
    assert_eq!(final_span.style.fg, Some(theme.text));
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
        .position(|line| line.contains("Read File") && line.contains("README.md"))
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
fn streaming_assistant_delta_buffers_then_reveals_with_gradient() {
    let mut timeline = TimelineState::default();
    let start = Instant::now();
    timeline.push_assistant_delta_streaming_at("hello", None, start);

    let initial = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 80, 8));
    let initial_text = initial
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(!initial_text.contains("hello"));
    assert!(timeline.has_streaming_animation());

    assert!(timeline.tick_streaming_animation(start + Duration::from_millis(700), 80));
    let rendered = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 80, 8));
    let text = rendered
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(text.contains("hello"));
    assert!(
        rendered
            .text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.style.fg == Some(Theme::for_dark_background(true).accent))
    );
}

#[test]
fn streaming_commentary_delta_uses_commentary_text_without_accent_fade() {
    let mut timeline = TimelineState::default();
    let start = Instant::now();
    timeline.push_assistant_delta_streaming_at(
        "I will inspect first.",
        Some("commentary".to_string()),
        start,
    );

    assert!(timeline.tick_streaming_animation(start + Duration::from_millis(700), 80));
    let theme = Theme::for_dark_background(true);
    let rendered = timeline.render(theme, Rect::new(0, 0, 80, 8));
    let colors = rendered
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter_map(|span| span.style.fg)
        .collect::<Vec<_>>();

    assert!(colors.contains(&theme.commentary));
    assert!(!colors.contains(&theme.accent));
    assert!(!colors.contains(&theme.accent_soft));
}

#[test]
fn streaming_assistant_flushes_before_tool_rows() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta_streaming_at("I will inspect first.", None, Instant::now());
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );

    let lines = rendered_lines(&mut timeline);
    assert!(lines.iter().any(|line| line == "I will inspect first."));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Read File") && line.contains("README.md"))
    );
    assert!(!timeline.has_streaming_animation());
}

#[test]
fn streaming_reasoning_delta_buffers_with_neutral_fade() {
    let mut timeline = TimelineState::default();
    let start = Instant::now();
    timeline.push_reasoning_delta_streaming_at_for_test("thinking through it", start);

    let initial = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 80, 8));
    let initial_text = initial
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(!initial_text.contains("thinking through it"));
    assert!(timeline.has_streaming_animation());

    assert!(timeline.tick_streaming_animation(start + Duration::from_millis(700), 80));
    let theme = Theme::for_dark_background(true);
    let rendered = timeline.render(theme, Rect::new(0, 0, 80, 8));
    let text = rendered
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(text.contains("thinking through it"));
    let fade_colors = rendered
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter_map(|span| span.style.fg)
        .collect::<Vec<_>>();
    assert!(!fade_colors.contains(&theme.accent));
    assert!(!fade_colors.contains(&theme.accent_soft));
    assert!(fade_colors.contains(&theme.text));
}

#[test]
fn applying_minimal_theme_removes_thinking_lines_from_timeline() {
    // Headline RFC demo: `.timeline-thinking { display: none; }` removes the
    // chain-of-thought block. We build the same transcript twice: once with
    // the baseline theme, once with minimal.css applied. The thinking line
    // exists in the first, vanishes in the second.
    let mut timeline = TimelineState::default();
    timeline.push_user("hello");
    timeline.push_reasoning_delta("I am pondering.");
    timeline.push_assistant_delta("hi", Some("final_answer".to_string()));

    let baseline = Theme::for_dark_background(true);
    let baseline_lines = timeline
        .render(baseline, Rect::new(0, 0, 100, 20))
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
    assert!(baseline_lines.iter().any(|l| l.contains("I am pondering")));

    let mut themed = baseline;
    themed.hide_thinking = true;
    let themed_lines = timeline
        .render(themed, Rect::new(0, 0, 100, 20))
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
    assert!(!themed_lines.iter().any(|l| l.contains("I am pondering")));
    assert!(themed_lines.iter().any(|l| l.contains("hi")));
}

#[test]
fn loading_minimal_theme_from_disk_flips_hide_thinking() {
    use crate::theme::overrides::ThemeOverrides;
    let css = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("themes")
            .join("minimal.css"),
    )
    .expect("minimal.css should exist");
    let overrides = ThemeOverrides::from_css(&css).expect("parse");
    assert!(overrides.hides("timeline-thinking"));
}

#[test]
fn reasoning_deltas_render_live_as_thinking() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("The user is asking for ");
    timeline.push_reasoning_delta("visible thinking tokens.");
    timeline.push_assistant_delta("Done.", Some("final_answer".to_string()));

    let theme = Theme::for_dark_background(true);
    let render = timeline.render(theme, Rect::new(0, 0, 100, 20));
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
            .any(|line| line.contains("The user is asking for visible thinking tokens."))
    );
    assert!(lines.iter().any(|line| line == "Done."));

    let reasoning_span = render
        .text
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.as_ref() == "The user is asking for visible thinking tokens.")
        .expect("reasoning span should render");
    assert_eq!(reasoning_span.style.fg, Some(theme.thinking));
    assert_eq!(reasoning_span.style.bg, Some(theme.user_message_bg));
    assert!(reasoning_span.style.add_modifier.contains(Modifier::ITALIC));

    let reasoning_row = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref() == "The user is asking for visible thinking tokens."
            })
        })
        .expect("reasoning row should render");
    let row_text = reasoning_row
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(row_text.chars().count(), 100);
    assert!(row_text.starts_with("▌   The user is asking"));
    assert_eq!(reasoning_row.spans[0].style.fg, Some(theme.thinking));
    assert_eq!(reasoning_row.spans[0].style.bg, Some(theme.user_message_bg));
}

#[test]
fn reasoning_blocks_are_separated_by_blank_line_when_resumed() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("First thought.\n\nSecond thought.");

    let lines = rendered_lines(&mut timeline);
    let first_index = lines
        .iter()
        .position(|line| line.contains("First thought."))
        .expect("first reasoning block should render");
    let second_index = lines
        .iter()
        .position(|line| line.contains("Second thought."))
        .expect("second reasoning block should render");

    assert!(
        lines[first_index + 1]
            .trim_start_matches('▌')
            .trim()
            .is_empty()
    );
    assert_eq!(second_index, first_index + 2);
}

#[test]
fn reasoning_deltas_within_same_block_remain_adjacent() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("First chunk. ");
    timeline.push_reasoning_delta("Continued chunk.");

    let lines = rendered_lines(&mut timeline);
    let thought_index = lines
        .iter()
        .position(|line| line.contains("First chunk. Continued chunk."))
        .expect("single reasoning block should render on one line");
    assert!(
        !lines
            .iter()
            .skip(1)
            .take(thought_index)
            .any(|line| line.trim_start_matches('▌').trim().is_empty())
    );
}

#[test]
fn reasoning_heading_moves_to_working_status_and_is_removed_from_timeline() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta(
        "**Inspecting Code Modifications**\n\nI need to inspect the app.rs file.",
    );

    assert_eq!(
        timeline.latest_reasoning_heading().as_deref(),
        Some("Inspecting Code Modifications")
    );
    let lines = rendered_lines(&mut timeline);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("I need to inspect the app.rs file."))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Inspecting Code Modifications"))
    );
}

#[test]
fn timeline_only_renders_the_most_recent_reasoning_block() {
    let mut timeline = TimelineState::default();
    timeline.push_reasoning_delta("first hidden thought");
    timeline.push_assistant_delta("partial", Some("commentary".to_string()));
    timeline.push_reasoning_delta("second visible thought");

    let lines = rendered_lines(&mut timeline);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("first hidden thought"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("second visible thought"))
    );
}

#[test]
fn turn_completed_renders_right_aligned_usage_summary() {
    let mut timeline = TimelineState::default();
    timeline.push_turn_completed(TurnCompletedSummary {
        elapsed: Duration::from_millis(1234),
        input_tokens: 46_694,
        output_tokens: 1_240,
        reasoning_tokens: Some(512),
        thread_tokens: 100_200,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed in"))
        .expect("turn completion row should be rendered");

    assert!(row.starts_with("  Turn completed in 1.2 sec."));
    assert!(
        row.trim_end()
            .ends_with("↑ 46K  ↓ 1.2K  thinking 512  thread 100K tokens")
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
        thread_tokens: 420,
    });

    let lines = rendered_lines(&mut timeline);
    let row = lines
        .iter()
        .find(|line| line.contains("Turn completed in"))
        .expect("turn completion row should be rendered");

    assert_eq!(
        row.trim_end(),
        "  Turn completed in 2 min 5 sec.  ↑ 400  ↓ 20  thread 420 tokens"
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
    let mut timeline = TimelineState::new(
        ScrollSettings {
            acceleration_enabled: false,
            fixed_rows_per_tick: 10.0,
        },
        TimelineSettings::default(),
    );
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

    assert_eq!(usize::from(at_bottom - after_up), 10);

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
    assert!(visible.iter().any(|line| line.contains("line 4")));
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
fn mouse_button_up_on_expanded_tool_does_not_reopen_it() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("glob", r#"{"pattern":"**/*.rs"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("src/main.rs".to_string()));
    timeline.fold_state.set_expanded("call_1", true);
    timeline.selected = Some(0);
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 4, 100, 10));

    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 4,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        ..down
    };

    assert!(timeline.handle_mouse(down));
    assert!(timeline.handle_mouse(up));
    assert!(!timeline.fold_state.is_expanded("call_1"));
}

#[test]
fn tool_row_relative_times_are_human_readable_without_plus_or_leading_zeroes() {
    let mut timeline = TimelineState::default();
    for (id, command) in [
        ("call_1", "first"),
        ("call_2", "second"),
        ("call_3", "third"),
    ] {
        timeline.record_tool_requested(
            id.to_string(),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"{command}"}}"#)),
        );
    }

    let base = time::OffsetDateTime::UNIX_EPOCH;
    if let TimelineItemKind::Tool(tool) = &mut timeline.items[0].kind {
        tool.started_at = base;
    }
    if let TimelineItemKind::Tool(tool) = &mut timeline.items[1].kind {
        tool.started_at = base + time::Duration::seconds(7);
    }
    if let TimelineItemKind::Tool(tool) = &mut timeline.items[2].kind {
        tool.started_at = base + time::Duration::seconds(153);
    }

    let lines = rendered_lines(&mut timeline);
    assert!(lines[0].trim_end().ends_with("0s"));
    assert!(lines[1].trim_end().ends_with("7s"));
    assert!(lines[2].trim_end().ends_with("2m 26s"));
    assert!(!lines[1].contains("+07s"));
    assert!(!lines[2].contains("+153s"));
}

#[test]
fn running_shell_tool_renders_live_tail_as_raw_terminal_rows() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_shell".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"make test"}"#),
    );
    let output = (1..=15)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    timeline.record_tool_output_delta("call_shell", &output);

    let lines = rendered_lines(&mut timeline);

    let output_rows = lines
        .iter()
        .filter(|line| line.starts_with("line "))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(output_rows.len(), 15);
    assert!(!output_rows.iter().any(|line| line.contains('↳')));
    assert!(output_rows.iter().any(|line| line.contains("line 1")));
    assert!(output_rows.iter().any(|line| line.contains("line 15")));
}

#[test]
fn expanded_shell_tool_scrolls_completed_output_to_bottom() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_shell".to_string(),
        ToolTimelineEntry::new("exec_command", r#"{"cmd":"npm test"}"#),
    );
    let output = (1..=40)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    timeline.record_tool_completed("call_shell", false, Some(output));
    timeline.fold_state.set_expanded("call_shell", true);

    let lines = timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 80))
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

    let output_rows = lines
        .iter()
        .filter(|line| line.starts_with("line "))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(output_rows.len(), 24);
    assert!(!output_rows.iter().any(|line| line.ends_with("line 1")));
    assert!(!output_rows.iter().any(|line| line.ends_with("line 16")));
    assert!(output_rows.iter().any(|line| line.ends_with("line 17")));
    assert!(output_rows.iter().any(|line| line.ends_with("line 40")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("output scrolled") && line.contains("16 earlier lines"))
    );
}

#[test]
fn shell_output_renders_truecolor_ansi_without_gutter() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_shell".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"printf"}"#),
    );
    timeline.record_tool_output_delta("call_shell", "plain \u{1b}[38;2;12;34;56mrgb\u{1b}[0m tail");

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 20));
    let line = render
        .text
        .lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .contains("plain rgb tail")
        })
        .expect("terminal output should render");

    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(rendered, "plain rgb tail");
    assert!(!rendered.contains('↳'));
    assert!(line.spans.iter().any(|span| {
        span.content.as_ref() == "rgb" && span.style.fg == Some(Color::Rgb(12, 34, 56))
    }));
}

#[test]
fn stdin_session_updates_append_to_original_exec_row() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_exec".to_string(),
        ToolTimelineEntry::new("exec_command", r#"{"cmd":"npm test"}"#),
    );
    timeline.record_tool_completed("call_exec", false, Some("first chunk".to_string()));

    timeline.record_tool_session_update("call_exec", false, Some("second chunk".to_string()), true);

    assert_eq!(
        timeline
            .items
            .iter()
            .filter(|item| matches!(item.kind, TimelineItemKind::Tool(_)))
            .count(),
        1
    );
    let lines = rendered_lines(&mut timeline);
    assert!(lines.iter().any(|line| line.contains("Exec Command")));
    assert!(lines.iter().any(|line| line.contains("first chunk")));
    assert!(lines.iter().any(|line| line.contains("second chunk")));
    assert!(!lines.iter().any(|line| line.contains("Write Stdin")));
}

#[test]
fn new_shell_tool_collapses_previous_shell_output() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_one".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"first"}"#),
    );
    timeline.record_tool_completed("call_one", false, Some("previous output".to_string()));
    timeline.fold_state.set_expanded("call_one", true);

    timeline.record_tool_requested(
        "call_two".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"second"}"#),
    );

    assert!(!timeline.fold_state.is_expanded("call_one"));
    let lines = rendered_lines(&mut timeline);
    assert!(!lines.iter().any(|line| line.contains("previous output")));
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
    assert_eq!(detail.tool_id.as_deref(), Some("call_1"));
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
fn clicking_error_row_requests_detail_modal_with_body() {
    let mut timeline = TimelineState::default();
    timeline.push_error(
        "Synthetic Chat Completions error 502 Bad Gateway: provider server error\n\n--- response body ---\n{\"error\":\"upstream timeout\"}",
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
        .expect("error row click should request detail modal");
    assert!(detail.failed);
    assert!(detail.command.is_none());
    assert!(detail.output.as_deref().unwrap().contains("502 Bad Gateway"));
    assert!(detail.output.as_deref().unwrap().contains("upstream timeout"));
}

#[test]
fn enter_on_selected_error_row_requests_detail_modal() {
    let mut timeline = TimelineState::default();
    timeline.push_error("Synthetic Chat Completions error 502: provider server error");
    // Select the error row (it's the only non-message selectable item).
    timeline.selected = Some(0);
    timeline.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));

    let detail = timeline
        .take_requested_detail()
        .expect("Enter on error row should request detail modal");
    assert!(detail.failed);
    assert!(detail.output.as_deref().unwrap().contains("502"));
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

    let row = timeline
        .hit_rows
        .iter()
        .find_map(|(row, index)| (*index == 1).then_some(*row))
        .expect("second tool should have a visible hit row");

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    assert_eq!(timeline.selected, Some(1));
}

#[test]
fn timeline_emits_transcript_and_tool_regions_for_visible_rows() {
    let mut timeline = TimelineState::default();
    timeline.push_user("inspect this");
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    let area = Rect::new(0, 2, 80, 10);
    timeline.render(Theme::for_dark_background(true), area);

    let regions = timeline.interactive_regions(area, "thread-a", "turn-a");

    assert!(
        regions.iter().any(|region| matches!(
            &region.kind,
            RegionKind::TranscriptMessage {
                thread_id,
                turn_id,
                message_idx: 0,
            } if thread_id == "thread-a" && turn_id == "turn-a"
        )),
        "missing transcript message region: {regions:?}"
    );
    assert!(
        regions.iter().any(|region| matches!(
            &region.kind,
            RegionKind::ToolCallBlock {
                call_id,
                expanded: false,
            } if call_id == "call_1"
        )),
        "missing tool call region: {regions:?}"
    );
}

#[test]
fn tool_fold_state_is_keyed_by_stable_call_id() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("contents".to_string()));
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 3, 80, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 3,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(click));
    assert!(timeline.handle_mouse(click));

    let regions = timeline.interactive_regions(Rect::new(0, 3, 80, 10), "thread-a", "turn-a");
    assert!(regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::ToolCallBlock {
            call_id,
            expanded: true,
        } if call_id == "call_1"
    )));
}

#[test]
fn tool_fold_state_persists_through_thread_scoped_extension_state() {
    let mut timeline = TimelineState::default();
    timeline.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    timeline.record_tool_completed("call_1", false, Some("contents".to_string()));
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 3, 80, 10));

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 3,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(click));
    assert!(timeline.handle_mouse(click));

    let record = timeline.fold_state_record("thread-a").unwrap();
    assert_eq!(
        record.scope,
        ExtensionStoreScope::Thread {
            thread_id: "thread-a".to_string()
        }
    );

    let mut resumed = TimelineState::default();
    resumed.record_tool_requested(
        "call_1".to_string(),
        ToolTimelineEntry::new("read_file", r#"{"path":"README.md"}"#),
    );
    resumed.record_tool_completed("call_1", false, Some("contents".to_string()));
    resumed
        .restore_fold_state_record(&record, "thread-a")
        .unwrap();
    resumed.render(Theme::for_dark_background(true), Rect::new(0, 3, 80, 10));

    let regions = resumed.interactive_regions(Rect::new(0, 3, 80, 10), "thread-a", "turn-a");
    assert!(regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::ToolCallBlock {
            call_id,
            expanded: true,
        } if call_id == "call_1"
    )));
}

#[test]
fn timeline_emits_url_and_file_reference_regions_from_transcript_text() {
    let mut timeline = TimelineState::default();
    timeline.push_system("See https://example.com and crates/roder-tui/src/app.rs:42");
    let area = Rect::new(0, 2, 100, 10);
    timeline.render(Theme::for_dark_background(true), area);

    let regions = timeline.interactive_regions(area, "thread-a", "turn-a");

    assert!(regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::Url(url) if url == "https://example.com"
    )));
    assert!(regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::FileReference { path, line: Some(42) }
            if path == std::path::Path::new("crates/roder-tui/src/app.rs")
    )));
}

#[test]
fn long_message_folding_is_disabled_by_default() {
    let mut timeline = TimelineState::default();
    timeline.push_assistant_delta(
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten",
        None,
    );
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 2, 80, 12));

    let unfolded = rendered_lines(&mut timeline);
    assert!(!unfolded.iter().any(|line| line.contains("2 more lines")));
    assert!(unfolded.iter().any(|line| line.contains("nine")));

    assert_eq!(timeline.selected, None);
}

#[test]
fn long_message_folding_can_be_enabled() {
    let mut timeline = TimelineState::new(
        ScrollSettings::default(),
        TimelineSettings {
            message_folding: true,
        },
    );
    timeline.push_assistant_delta(
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten",
        None,
    );

    let collapsed = rendered_lines(&mut timeline);

    assert!(collapsed.iter().any(|line| line.contains("2 more lines")));
    assert!(!collapsed.iter().any(|line| line.contains("nine")));

    timeline.render(Theme::for_dark_background(true), Rect::new(0, 2, 80, 12));
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 2,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(click));

    let expanded = rendered_lines(&mut timeline);
    assert!(expanded.iter().any(|line| line.contains("nine")));
    assert!(!expanded.iter().any(|line| line.contains("more lines")));
}

#[test]
fn timeline_virtualization_render_bounds_rendered_text_to_visible_window() {
    let mut timeline = TimelineState::default();
    for index in 0..1_000 {
        timeline.push_system(format!("event {index}"));
    }

    let render = timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 6));

    assert!(render.scroll > 0);
    assert!(
        render.text.lines.len() <= 18,
        "rendered {} lines instead of a viewport-sized window",
        render.text.lines.len()
    );
    assert!(
        visible_lines(&mut timeline, 6)
            .iter()
            .any(|line| line.contains("event 999"))
    );
}

#[test]
fn timeline_virtualization_render_keeps_bottom_padding_at_end() {
    let mut timeline = TimelineState::default();
    timeline.push_system("last visible event");

    let visible = visible_lines(&mut timeline, 4);

    assert!(
        visible
            .iter()
            .any(|line| line.contains("last visible event"))
    );
    assert_eq!(visible.len(), 4);
    assert_eq!(visible[1], "");
    assert_eq!(visible[2], "");
    assert_eq!(visible[3], "");
}

#[test]
fn timeline_virtualization_interaction_keeps_visible_hit_rows_clickable() {
    let mut timeline = TimelineState::default();
    for index in 0..20 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("shell", format!(r#"{{"command":"echo {index}"}}"#)),
        );
    }
    let area = Rect::new(0, 0, 100, 5);
    timeline.render(Theme::for_dark_background(true), area);

    let visible_rows = timeline.hit_rows.clone();
    assert!(!visible_rows.is_empty());
    let (row, index) = *visible_rows.last().expect("visible row");
    assert!(index < timeline.items.len());

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };
    assert!(timeline.handle_mouse(click));
    assert_eq!(timeline.selected, Some(index));
}

#[test]
fn timeline_virtualization_interaction_does_not_click_offscreen_items() {
    let mut timeline = TimelineState::default();
    for index in 0..30 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("shell", format!(r#"{{"command":"echo {index}"}}"#)),
        );
    }
    let area = Rect::new(0, 10, 100, 4);
    timeline.render(Theme::for_dark_background(true), area);

    let offscreen_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(!timeline.handle_mouse(offscreen_click));
    assert_eq!(timeline.selected, None);
}

#[test]
fn timeline_virtualization_interaction_overflow_row_still_reveals_tools() {
    let mut timeline = TimelineState::default();
    for index in 0..8 {
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("read_file", format!(r#"{{"path":"file-{index}.rs"}}"#)),
        );
    }
    timeline.focus_latest();
    timeline.handle_key(key(KeyCode::Home));
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 5));

    let overflow_row = timeline
        .hit_rows
        .iter()
        .find_map(|(row, index)| (*index == TOOL_OVERFLOW_INDEX).then_some(*row))
        .expect("overflow row should be visible");
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: overflow_row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(click));
    assert!(timeline.show_all_tools);
}

#[test]
fn timeline_virtualization_interaction_auto_follow_uses_virtual_scroll() {
    let mut timeline = TimelineState::default();
    for index in 0..12 {
        timeline.push_system(format!("event {index}"));
    }
    let area = Rect::new(0, 0, 100, 5);
    let first_scroll = timeline
        .render(Theme::for_dark_background(true), area)
        .scroll;

    timeline.push_system("latest event");
    let followed = timeline.render(Theme::for_dark_background(true), area);

    assert!(followed.scroll > first_scroll);
    assert!(
        visible_lines(&mut timeline, 5)
            .iter()
            .any(|line| line.contains("latest event"))
    );
}

#[test]
fn timeline_reuses_cached_static_render_while_scrolling() {
    let mut timeline = TimelineState::default();
    for index in 0..80 {
        timeline.push_assistant_delta(&format!("assistant message {index}"), None);
    }

    let area = Rect::new(0, 0, 80, 8);
    let first = timeline.render(Theme::for_dark_background(true), area);
    assert!(first.scroll > 0);
    assert!(timeline.render_cache.is_some());

    timeline.focus_latest();
    assert!(timeline.handle_key(key(KeyCode::PageUp)));
    let after_scroll = timeline.render(Theme::for_dark_background(true), area);

    assert!(timeline.render_cache.is_some());
    assert!(after_scroll.scroll < first.scroll);
}

#[test]
fn timeline_static_render_cache_invalidates_on_new_item() {
    let mut timeline = TimelineState::default();
    for index in 0..20 {
        timeline.push_system(format!("event {index}"));
    }

    timeline.render(Theme::for_dark_background(true), Rect::new(0, 0, 80, 8));
    assert!(timeline.render_cache.is_some());

    timeline.push_system("new event");

    assert!(timeline.render_cache.is_none());
    let visible = visible_lines(&mut timeline, 8);
    assert!(visible.iter().any(|line| line.contains("new event")));
}

#[test]
fn long_timeline_virtualization_bounds_mixed_transcript_output() {
    let mut timeline = TimelineState::default();
    for index in 0..250 {
        timeline.push_user(format!("user message {index}"));
        timeline.push_assistant_delta(&format!("assistant message {index}"), None);
        timeline.record_tool_requested(
            format!("call_{index}"),
            ToolTimelineEntry::new("grep", format!(r#"{{"query":"{index}","path":"src"}}"#)),
        );
    }
    timeline.record_tool_requested(
        "call_expand".to_string(),
        ToolTimelineEntry::new("shell", r#"{"command":"printf expanded"}"#),
    );
    timeline.record_tool_completed(
        "call_expand",
        false,
        Some("expanded output\nsecond line".to_string()),
    );
    timeline.fold_state.toggle("call_expand".to_string());
    timeline.record_tool_requested(
        "call_diff".to_string(),
        ToolTimelineEntry::new("apply_patch", ""),
    );
    timeline.record_tool_delta(
        "call_diff",
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
        render.scroll > 700,
        "scroll should reflect the full transcript height"
    );
    assert!(
        lines.len() <= 36,
        "rendered {} lines instead of a bounded viewport window",
        lines.len()
    );
    assert!(lines.iter().any(|line| line.contains("expanded output")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("src/lib.rs") && line.contains("+1") && line.contains("-1"))
    );
}

#[test]
fn right_click_transcript_message_opens_context_menu_regions_near_edges() {
    let mut timeline = TimelineState::default();
    timeline.push_system("copyable message");
    let area = Rect::new(0, 0, 40, 5);
    timeline.render(Theme::for_dark_background(true), area);

    let right_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 39,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::empty(),
    };

    assert!(timeline.handle_mouse(right_click));
    let regions = timeline.interactive_regions(area, "thread-a", "turn-a");
    let menu_regions = regions
        .iter()
        .filter(|region| {
            matches!(
                &region.kind,
                RegionKind::Custom { extension_id, .. }
                    if extension_id == "roder-tui/transcript-context-menu"
            )
        })
        .collect::<Vec<_>>();

    assert!(!menu_regions.is_empty());
    assert!(
        menu_regions
            .iter()
            .all(|region| region.rect.x + region.rect.width <= area.width)
    );
    assert!(menu_regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::Custom { payload, .. }
            if payload["action"] == serde_json::json!("copy")
    )));
    assert!(menu_regions.iter().any(|region| matches!(
        &region.kind,
        RegionKind::Custom { payload, .. }
            if payload["action"] == serde_json::json!("jump_to_tool_call")
    )));
}
