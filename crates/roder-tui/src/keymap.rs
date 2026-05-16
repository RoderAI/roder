use std::collections::{BTreeMap, BTreeSet, HashMap};

use roder_api::interactive::{RegionId, RegionKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    ExpandToolCall,
    CollapseToolCall,
    OpenUrl,
    OpenFileRef,
    FoldMessage,
    OpenContextMenu,
    CopySelection,
    PasteToComposer,
    ApproveHunk,
    RejectHunk,
    OpenPalette,
    CycleMode,
    ScrollTranscript,
    FocusNextRegion,
    FocusPreviousRegion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: String,
    pub modifiers: BTreeSet<KeyModifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyModifier {
    Control,
    Shift,
    Alt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<Action, Vec<KeyBinding>>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut keymap = Self {
            bindings: BTreeMap::new(),
        };
        keymap.bind(Action::ExpandToolCall, KeyBinding::plain("enter"));
        keymap.bind(Action::CollapseToolCall, KeyBinding::plain("enter"));
        keymap.bind(Action::OpenUrl, KeyBinding::plain("enter"));
        keymap.bind(Action::OpenFileRef, KeyBinding::plain("enter"));
        keymap.bind(Action::FoldMessage, KeyBinding::plain("enter"));
        keymap.bind(
            Action::OpenContextMenu,
            KeyBinding::modified("f10", [KeyModifier::Shift]),
        );
        keymap.bind(Action::CopySelection, KeyBinding::plain("c"));
        keymap.bind(Action::PasteToComposer, KeyBinding::plain("p"));
        keymap.bind(Action::ApproveHunk, KeyBinding::plain("y"));
        keymap.bind(Action::RejectHunk, KeyBinding::plain("n"));
        keymap.bind(
            Action::OpenPalette,
            KeyBinding::modified("k", [KeyModifier::Control]),
        );
        keymap.bind(Action::CycleMode, KeyBinding::plain("backtab"));
        keymap.bind(Action::ScrollTranscript, KeyBinding::plain("pagedown"));
        keymap.bind(Action::ScrollTranscript, KeyBinding::plain("pageup"));
        keymap.bind(Action::FocusNextRegion, KeyBinding::plain("tab"));
        keymap.bind(
            Action::FocusPreviousRegion,
            KeyBinding::modified("tab", [KeyModifier::Shift]),
        );
        keymap
    }
}

impl Keymap {
    pub fn with_overrides(overrides: &HashMap<String, Vec<String>>) -> Self {
        let mut keymap = Self::default();
        for (action, bindings) in overrides {
            let Some(action) = action_from_id(action) else {
                continue;
            };
            let parsed = bindings
                .iter()
                .filter_map(|binding| parse_binding(binding))
                .collect::<Vec<_>>();
            if !parsed.is_empty() {
                keymap.bindings.insert(action, parsed);
            }
        }
        keymap
    }

    pub fn bind(&mut self, action: Action, binding: KeyBinding) {
        self.bindings.entry(action).or_default().push(binding);
    }

    pub fn bindings_for(&self, action: Action) -> &[KeyBinding] {
        self.bindings
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn actions(&self) -> BTreeSet<Action> {
        self.bindings.keys().copied().collect()
    }
}

impl KeyBinding {
    pub fn plain(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            modifiers: BTreeSet::new(),
        }
    }

    pub fn modified(
        key: impl Into<String>,
        modifiers: impl IntoIterator<Item = KeyModifier>,
    ) -> Self {
        Self {
            key: key.into(),
            modifiers: modifiers.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusRing {
    regions: Vec<RegionId>,
    selected: Option<usize>,
}

impl FocusRing {
    pub fn new(regions: impl IntoIterator<Item = RegionId>) -> Self {
        let regions = regions.into_iter().collect::<Vec<_>>();
        Self {
            selected: (!regions.is_empty()).then_some(0),
            regions,
        }
    }

    pub fn current(&self) -> Option<&RegionId> {
        self.selected.and_then(|idx| self.regions.get(idx))
    }

    pub fn focus_next(&mut self) -> Option<&RegionId> {
        if self.regions.is_empty() {
            self.selected = None;
            return None;
        }
        let next = self
            .selected
            .map(|idx| (idx + 1) % self.regions.len())
            .unwrap_or(0);
        self.selected = Some(next);
        self.current()
    }

    pub fn focus_previous(&mut self) -> Option<&RegionId> {
        if self.regions.is_empty() {
            self.selected = None;
            return None;
        }
        let current = self.selected.unwrap_or(0);
        self.selected = Some(if current == 0 {
            self.regions.len() - 1
        } else {
            current - 1
        });
        self.current()
    }
}

pub fn actions_for_region_kind(kind: &RegionKind) -> BTreeSet<Action> {
    let actions = match kind {
        RegionKind::ToolCallBlock { .. } => {
            vec![Action::ExpandToolCall, Action::CollapseToolCall]
        }
        RegionKind::FileReference { .. } => vec![Action::OpenFileRef, Action::OpenContextMenu],
        RegionKind::Url(_) => vec![Action::OpenUrl, Action::OpenContextMenu],
        RegionKind::TranscriptMessage { .. } => vec![Action::FoldMessage, Action::OpenContextMenu],
        RegionKind::DiffHunk { .. } => vec![Action::ApproveHunk, Action::RejectHunk],
        RegionKind::PolicyApprovalButton { .. } => {
            vec![Action::ApproveHunk, Action::RejectHunk]
        }
        RegionKind::PaletteItem { .. } => vec![Action::OpenPalette],
        RegionKind::Composer => vec![Action::PasteToComposer],
        RegionKind::StatusSegment { .. }
        | RegionKind::AttachmentThumbnail { .. }
        | RegionKind::Custom { .. } => vec![Action::FocusNextRegion],
    };
    actions.into_iter().collect()
}

pub fn missing_keyboard_parity(
    mouse_actions: impl IntoIterator<Item = Action>,
    keymap: &Keymap,
) -> BTreeSet<Action> {
    let keyboard_actions = keymap.actions();
    mouse_actions
        .into_iter()
        .filter(|action| !keyboard_actions.contains(action))
        .collect()
}

fn parse_binding(binding: &str) -> Option<KeyBinding> {
    let mut parts = binding.split('+').map(|part| part.trim().to_lowercase());
    let mut modifiers = BTreeSet::new();
    let mut key = None;
    for part in parts.by_ref() {
        match part.as_str() {
            "ctrl" | "control" => {
                modifiers.insert(KeyModifier::Control);
            }
            "shift" => {
                modifiers.insert(KeyModifier::Shift);
            }
            "alt" | "option" => {
                modifiers.insert(KeyModifier::Alt);
            }
            "" => {}
            value => key = Some(value.to_string()),
        }
    }
    key.map(|key| KeyBinding { key, modifiers })
}

fn action_from_id(value: &str) -> Option<Action> {
    Some(match value {
        "expand_tool_call" => Action::ExpandToolCall,
        "collapse_tool_call" => Action::CollapseToolCall,
        "open_url" => Action::OpenUrl,
        "open_file_ref" => Action::OpenFileRef,
        "fold_message" => Action::FoldMessage,
        "open_context_menu" => Action::OpenContextMenu,
        "copy_selection" => Action::CopySelection,
        "paste_to_composer" => Action::PasteToComposer,
        "approve_hunk" => Action::ApproveHunk,
        "reject_hunk" => Action::RejectHunk,
        "open_palette" => Action::OpenPalette,
        "cycle_mode" => Action::CycleMode,
        "scroll_transcript" => Action::ScrollTranscript,
        "focus_next_region" => Action::FocusNextRegion,
        "focus_previous_region" => Action::FocusPreviousRegion,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_applies_config_overrides() {
        let overrides = HashMap::from([("open_palette".to_string(), vec!["ctrl+p".to_string()])]);

        let keymap = Keymap::with_overrides(&overrides);

        assert_eq!(
            keymap.bindings_for(Action::OpenPalette),
            &[KeyBinding::modified("p", [KeyModifier::Control])]
        );
    }

    #[test]
    fn focus_ring_traverses_regions_in_both_directions() {
        let mut ring = FocusRing::new(["a".to_string(), "b".to_string()]);

        assert_eq!(ring.current().map(String::as_str), Some("a"));
        assert_eq!(ring.focus_next().map(String::as_str), Some("b"));
        assert_eq!(ring.focus_next().map(String::as_str), Some("a"));
        assert_eq!(ring.focus_previous().map(String::as_str), Some("b"));
    }
}
