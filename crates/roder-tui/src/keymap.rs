use std::{collections::BTreeMap, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    ExpandToolCall,
    CollapseToolCall,
    OpenUrl,
    OpenFileRef,
    FoldMessage,
    CopySelection,
    PasteToComposer,
    ApproveHunk,
    RejectHunk,
    OpenPalette,
    CycleMode,
    FocusNextRegion,
    FocusPreviousRegion,
    ScrollTranscript,
    ScrollPalette,
    ScrollDiff,
    ScrollMonitor,
}

impl Action {
    pub fn id(self) -> &'static str {
        match self {
            Self::ExpandToolCall => "tool_call/expand",
            Self::CollapseToolCall => "tool_call/collapse",
            Self::OpenUrl => "url/open",
            Self::OpenFileRef => "file_ref/open",
            Self::FoldMessage => "message/fold",
            Self::CopySelection => "selection/copy",
            Self::PasteToComposer => "selection/paste_to_composer",
            Self::ApproveHunk => "diff/approve_hunk",
            Self::RejectHunk => "diff/reject_hunk",
            Self::OpenPalette => "palette/open",
            Self::CycleMode => "mode/cycle",
            Self::FocusNextRegion => "region/focus_next",
            Self::FocusPreviousRegion => "region/focus_previous",
            Self::ScrollTranscript => "scroll/transcript",
            Self::ScrollPalette => "scroll/palette",
            Self::ScrollDiff => "scroll/diff",
            Self::ScrollMonitor => "scroll/monitor",
        }
    }

    pub fn all() -> &'static [Action] {
        &[
            Self::ExpandToolCall,
            Self::CollapseToolCall,
            Self::OpenUrl,
            Self::OpenFileRef,
            Self::FoldMessage,
            Self::CopySelection,
            Self::PasteToComposer,
            Self::ApproveHunk,
            Self::RejectHunk,
            Self::OpenPalette,
            Self::CycleMode,
            Self::FocusNextRegion,
            Self::FocusPreviousRegion,
            Self::ScrollTranscript,
            Self::ScrollPalette,
            Self::ScrollDiff,
            Self::ScrollMonitor,
        ]
    }

    pub fn mouse_dispatchable() -> &'static [Action] {
        Self::all()
    }

    pub fn keyboard_dispatchable() -> &'static [Action] {
        Self::all()
    }
}

impl FromStr for Action {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Action::all()
            .iter()
            .copied()
            .find(|action| action.id() == value || format!("{action:?}") == value)
            .ok_or_else(|| format!("unknown keymap action {value:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: String,
}

impl KeyBinding {
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBindingOverride {
    pub action: Action,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<Action, Vec<KeyBinding>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl Keymap {
    pub fn with_defaults() -> Self {
        let mut bindings = BTreeMap::<Action, Vec<KeyBinding>>::new();
        for (action, keys) in [
            (Action::ExpandToolCall, &["enter"][..]),
            (Action::CollapseToolCall, &["enter"][..]),
            (Action::OpenUrl, &["enter", "o"][..]),
            (Action::OpenFileRef, &["enter", "o"][..]),
            (Action::FoldMessage, &["enter"][..]),
            (Action::CopySelection, &["c"][..]),
            (Action::PasteToComposer, &["p"][..]),
            (Action::ApproveHunk, &["a"][..]),
            (Action::RejectHunk, &["r"][..]),
            (Action::OpenPalette, &["ctrl+p"][..]),
            (Action::CycleMode, &["shift+tab"][..]),
            (Action::FocusNextRegion, &["tab"][..]),
            (Action::FocusPreviousRegion, &["shift+tab"][..]),
            (Action::ScrollTranscript, &["wheel"][..]),
            (Action::ScrollPalette, &["wheel"][..]),
            (Action::ScrollDiff, &["wheel"][..]),
            (Action::ScrollMonitor, &["wheel"][..]),
        ] {
            bindings.insert(
                action,
                keys.iter().map(|key| KeyBinding::new(*key)).collect(),
            );
        }
        Self { bindings }
    }

    pub fn with_overrides(
        mut self,
        overrides: impl IntoIterator<Item = KeyBindingOverride>,
    ) -> Self {
        for override_binding in overrides {
            self.bindings.insert(
                override_binding.action,
                override_binding
                    .keys
                    .into_iter()
                    .map(KeyBinding::new)
                    .collect(),
            );
        }
        self
    }

    pub fn bindings_for(&self, action: Action) -> &[KeyBinding] {
        self.bindings.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn action_ids_cover_interactive_intents_and_are_unique() {
        let ids = Action::all()
            .iter()
            .map(|action| action.id())
            .collect::<BTreeSet<_>>();

        assert_eq!(ids.len(), Action::all().len());
        for required in [
            "tool_call/expand",
            "tool_call/collapse",
            "url/open",
            "file_ref/open",
            "message/fold",
            "selection/copy",
            "selection/paste_to_composer",
            "diff/approve_hunk",
            "diff/reject_hunk",
            "palette/open",
            "mode/cycle",
            "region/focus_next",
            "region/focus_previous",
        ] {
            assert!(ids.contains(required), "missing action id {required}");
        }
    }

    #[test]
    fn default_keymap_covers_every_action() {
        let keymap = Keymap::default();

        for action in Action::all() {
            assert!(
                !keymap.bindings_for(*action).is_empty(),
                "missing binding for {}",
                action.id()
            );
        }
    }

    #[test]
    fn keymap_overrides_replace_default_bindings() {
        let keymap = Keymap::default().with_overrides([KeyBindingOverride {
            action: Action::OpenPalette,
            keys: vec!["ctrl+k".to_string()],
        }]);

        assert_eq!(
            keymap.bindings_for(Action::OpenPalette),
            &[KeyBinding::new("ctrl+k")]
        );
    }
}
