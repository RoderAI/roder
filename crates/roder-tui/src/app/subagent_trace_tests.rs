use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use roder_api::trace::{
    PagedTraceText, ParentTurnRef, SubagentDestination, SubagentDestinationKind,
    SubagentTraceDelta, SubagentTraceItem, SubagentTraceStatus, SubagentTraceSummary,
};

use super::Theme;
use super::tool_timeline::TimelineState;

fn summary(status: SubagentTraceStatus) -> SubagentTraceSummary {
    SubagentTraceSummary {
        trace_id: "trace-1".to_string(),
        parent: ParentTurnRef {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        },
        child_thread_id: "child-thread".to_string(),
        child_turn_id: "child-turn".to_string(),
        title: "Inspect repository".to_string(),
        role: "explore".to_string(),
        model: Some("mock".to_string()),
        lane: None,
        status,
        elapsed_ms: 1200,
        usage: None,
        destination: Some(SubagentDestination {
            kind: SubagentDestinationKind::InProcess,
            label: "in-process".to_string(),
            path: None,
            provider_id: None,
            destination_id: None,
        }),
        latest_activity: None,
        error_summary: None,
        exit_reason: None,
    }
}

fn rendered_lines(timeline: &mut TimelineState) -> Vec<String> {
    timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 16))
        .text
        .lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn subagent_trace_rows_render_and_expand() {
    let mut timeline = TimelineState::default();
    timeline.record_subagent_trace_created(summary(SubagentTraceStatus::Running));
    timeline.record_subagent_trace_delta(SubagentTraceDelta {
        trace_id: "trace-1".to_string(),
        parent: ParentTurnRef {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        },
        item: SubagentTraceItem::Message {
            role: "assistant".to_string(),
            content: PagedTraceText::capped("found src/lib.rs", 4000),
        },
    });

    let collapsed = rendered_lines(&mut timeline);
    assert!(
        collapsed
            .iter()
            .any(|line| line.contains("explore: Inspect repository"))
    );
    let collapsed_hits = collapsed
        .iter()
        .filter(|line| line.contains("found src/lib.rs"))
        .count();

    timeline.focus_latest();
    assert!(timeline.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    let expanded = rendered_lines(&mut timeline);
    let expanded_hits = expanded
        .iter()
        .filter(|line| line.contains("found src/lib.rs"))
        .count();
    assert!(expanded_hits > collapsed_hits);
}

#[test]
fn subagent_trace_rows_support_keyboard_and_mouse_selection() {
    let mut timeline = TimelineState::default();
    timeline.record_subagent_trace_created(summary(SubagentTraceStatus::Queued));
    timeline.record_subagent_trace_created(SubagentTraceSummary {
        trace_id: "trace-2".to_string(),
        title: "Review patch".to_string(),
        ..summary(SubagentTraceStatus::Queued)
    });
    timeline.render(Theme::for_dark_background(true), Rect::new(0, 2, 100, 10));

    timeline.focus_latest();
    assert!(timeline.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
    let lines = rendered_lines(&mut timeline);
    assert!(lines.iter().any(|line| line.contains("Inspect repository")));

    let clicked = (0..12).any(|row| {
        timeline.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row,
            modifiers: KeyModifiers::NONE,
        })
    });
    assert!(clicked);
}
