use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionCommand {
    Copy,
    Paste,
}

impl SelectionCommand {
    pub fn id(self) -> &'static str {
        match self {
            Self::Copy => "selection/copy",
            Self::Paste => "selection/paste",
        }
    }
}

pub fn selection_command_for_key(key: KeyEvent) -> Option<SelectionCommand> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    match key.code {
        KeyCode::Char('c') | KeyCode::Char('C') => Some(SelectionCommand::Copy),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(SelectionCommand::Paste),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_keys_map_to_stable_command_ids() {
        assert_eq!(
            selection_command_for_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
                .map(SelectionCommand::id),
            Some("selection/copy")
        );
        assert_eq!(
            selection_command_for_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
                .map(SelectionCommand::id),
            Some("selection/paste")
        );
    }

    #[test]
    fn modified_keys_do_not_shadow_existing_shortcuts() {
        assert_eq!(
            selection_command_for_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            None
        );
    }
}
