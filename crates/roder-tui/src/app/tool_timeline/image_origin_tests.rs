use super::*;

#[test]
fn new_team_timelines_default_unknown_instead_of_ui_cwd() {
    let mut timeline = TimelineState::default();
    assert_eq!(timeline.image_origin, ImageOrigin::Unknown);
    timeline.record_tool_requested(
        "view".into(),
        ToolTimelineEntry::new("view_image", r#"{"path":"/tmp/ui-host-only.png"}"#),
    );
    timeline.record_tool_completed("view", false, None);
    let text = timeline
        .render(Theme::for_terminal(), Rect::new(0, 0, 120, 30))
        .text
        .to_string();
    assert!(
        !text.contains("image ("),
        "unknown tool origins must not cause host reads"
    );
}

#[test]
fn explicit_local_origin_is_used_for_timeline_view_previews() {
    let mut timeline = TimelineState::default();
    timeline.set_image_origin(ImageOrigin::local("/other-thread".into()));
    timeline.record_tool_requested(
        "view".into(),
        ToolTimelineEntry::new("view_image", r#"{"path":"screenshot.png"}"#),
    );
    timeline.record_tool_completed("view", false, None);
    let text = timeline
        .render(Theme::for_terminal(), Rect::new(0, 0, 160, 30))
        .text
        .to_string();
    assert!(text.contains("/other-thread/screenshot.png"));
    timeline.set_image_origin(ImageOrigin::Remote);
    let text = timeline
        .render(Theme::for_terminal(), Rect::new(0, 0, 160, 30))
        .text
        .to_string();
    assert!(
        !text.contains("image ("),
        "retargeting must drop old-origin previews"
    );
}
