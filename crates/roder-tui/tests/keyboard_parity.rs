use std::collections::BTreeSet;

use roder_tui::keymap::Action;

#[test]
fn mouse_dispatchable_actions_have_keyboard_parity() {
    let mouse = Action::mouse_dispatchable()
        .iter()
        .map(|action| action.id())
        .collect::<BTreeSet<_>>();
    let keyboard = Action::keyboard_dispatchable()
        .iter()
        .map(|action| action.id())
        .collect::<BTreeSet<_>>();

    assert_eq!(mouse, keyboard);
}
