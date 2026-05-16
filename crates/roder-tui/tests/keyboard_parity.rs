use std::collections::BTreeSet;
use std::path::PathBuf;

use roder_api::interactive::{ApprovalVote, RegionKind};
use roder_tui::keymap::{
    Action, FocusRing, KeyBinding, KeyModifier, Keymap, actions_for_region_kind,
    missing_keyboard_parity,
};

#[test]
fn mouse_region_actions_have_keyboard_bindings() {
    let mouse_actions = representative_region_kinds()
        .iter()
        .flat_map(actions_for_region_kind)
        .collect::<BTreeSet<_>>();

    let missing = missing_keyboard_parity(mouse_actions, &Keymap::default());

    assert!(missing.is_empty(), "missing keyboard actions: {missing:?}");
}

#[test]
fn configurable_keymap_can_override_defaults() {
    let keymap = Keymap::with_overrides(&std::collections::HashMap::from([(
        "open_palette".to_string(),
        vec!["ctrl+p".to_string()],
    )]));

    assert_eq!(
        keymap.bindings_for(Action::OpenPalette),
        &[KeyBinding::modified("p", [KeyModifier::Control])]
    );
}

#[test]
fn focus_traversal_reaches_every_clickable_region() {
    let mut focus = FocusRing::new([
        "message-0".to_string(),
        "tool-call-call-1".to_string(),
        "url-0".to_string(),
        "file-0".to_string(),
    ]);

    assert_eq!(focus.current().map(String::as_str), Some("message-0"));
    assert_eq!(
        focus.focus_next().map(String::as_str),
        Some("tool-call-call-1")
    );
    assert_eq!(focus.focus_next().map(String::as_str), Some("url-0"));
    assert_eq!(focus.focus_next().map(String::as_str), Some("file-0"));
    assert_eq!(focus.focus_next().map(String::as_str), Some("message-0"));
    assert_eq!(focus.focus_previous().map(String::as_str), Some("file-0"));
}

fn representative_region_kinds() -> Vec<RegionKind> {
    vec![
        RegionKind::TranscriptMessage {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            message_idx: 0,
        },
        RegionKind::ToolCallBlock {
            call_id: "call".to_string(),
            expanded: true,
        },
        RegionKind::FileReference {
            path: PathBuf::from("src/main.rs"),
            line: Some(1),
        },
        RegionKind::Url("https://example.com".to_string()),
        RegionKind::DiffHunk {
            call_id: "call".to_string(),
            file_path: PathBuf::from("src/main.rs"),
            hunk_idx: 0,
        },
        RegionKind::PolicyApprovalButton {
            decision_id: "decision".to_string(),
            vote: ApprovalVote::Approve,
        },
        RegionKind::PaletteItem {
            source_id: "commands".to_string(),
            item_id: "run".to_string(),
        },
        RegionKind::Composer,
    ]
}
