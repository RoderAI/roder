use std::collections::{BTreeMap, BTreeSet, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as CrosstermKeyModifiers};
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

pub const HELP_ACTIONS: &[Action] = &[
    Action::OpenPalette,
    Action::CycleMode,
    Action::FocusNextRegion,
    Action::FocusPreviousRegion,
    Action::ScrollTranscript,
    Action::ExpandToolCall,
    Action::CollapseToolCall,
    Action::OpenUrl,
    Action::OpenFileRef,
    Action::FoldMessage,
    Action::OpenContextMenu,
    Action::CopySelection,
    Action::PasteToComposer,
    Action::ApproveHunk,
    Action::RejectHunk,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::ExpandToolCall => "Expand tool call",
            Action::CollapseToolCall => "Collapse tool call",
            Action::OpenUrl => "Open URL",
            Action::OpenFileRef => "Open file reference",
            Action::FoldMessage => "Fold message",
            Action::OpenContextMenu => "Open context menu",
            Action::CopySelection => "Copy selection",
            Action::PasteToComposer => "Paste to composer",
            Action::ApproveHunk => "Approve hunk",
            Action::RejectHunk => "Reject hunk",
            Action::OpenPalette => "Open palette",
            Action::CycleMode => "Cycle mode",
            Action::ScrollTranscript => "Scroll transcript",
            Action::FocusNextRegion => "Focus next region",
            Action::FocusPreviousRegion => "Focus previous region",
        }
    }
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
        keymap.bind(
            Action::CopySelection,
            KeyBinding::modified("c", [KeyModifier::Control, KeyModifier::Shift]),
        );
        keymap.bind(
            Action::PasteToComposer,
            KeyBinding::modified("v", [KeyModifier::Control, KeyModifier::Shift]),
        );
        keymap.bind(Action::ApproveHunk, KeyBinding::plain("y"));
        keymap.bind(Action::RejectHunk, KeyBinding::plain("n"));
        keymap.bind(
            Action::OpenPalette,
            KeyBinding::modified("k", [KeyModifier::Control]),
        );
        keymap.bind(
            Action::CycleMode,
            KeyBinding::modified("m", [KeyModifier::Control]),
        );
        keymap.bind(Action::ScrollTranscript, KeyBinding::plain("pagedown"));
        keymap.bind(Action::ScrollTranscript, KeyBinding::plain("pageup"));
        keymap.bind(Action::FocusNextRegion, KeyBinding::plain("tab"));
        keymap.bind(Action::FocusPreviousRegion, KeyBinding::plain("backtab"));
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

    pub fn binding_labels_for(&self, action: Action) -> Vec<String> {
        self.bindings_for(action)
            .iter()
            .map(KeyBinding::display_label)
            .collect()
    }

    pub fn actions(&self) -> BTreeSet<Action> {
        self.bindings.keys().copied().collect()
    }

    pub fn matches_key_event(&self, action: Action, key: &KeyEvent) -> bool {
        self.bindings_for(action)
            .iter()
            .any(|binding| binding.matches_key_event(key))
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

    pub fn matches_key_event(&self, event: &KeyEvent) -> bool {
        let key = self.normalized_key();
        let mut modifiers = key_modifiers_from_crossterm(event.modifiers);
        if key == "backtab" && event.code == KeyCode::BackTab {
            modifiers.remove(&KeyModifier::Shift);
        }
        key == normalized_key_code(event.code) && self.modifiers == modifiers
    }

    pub fn display_label(&self) -> String {
        let mut parts = self
            .modifiers
            .iter()
            .map(|modifier| match modifier {
                KeyModifier::Control => "Ctrl".to_string(),
                KeyModifier::Shift => "Shift".to_string(),
                KeyModifier::Alt => "Alt".to_string(),
            })
            .collect::<Vec<_>>();
        parts.push(display_key(&self.key));
        parts.join("+")
    }

    fn normalized_key(&self) -> String {
        self.key.to_lowercase()
    }
}

fn display_key(key: &str) -> String {
    match key.to_lowercase().as_str() {
        "enter" => "Enter".to_string(),
        "esc" => "Esc".to_string(),
        "tab" => "Tab".to_string(),
        "backtab" => "Shift+Tab".to_string(),
        "pagedown" => "PageDown".to_string(),
        "pageup" => "PageUp".to_string(),
        "f10" => "F10".to_string(),
        value if value.len() == 1 => value.to_uppercase(),
        value => value.to_string(),
    }
}

fn normalized_key_code(code: KeyCode) -> String {
    match code {
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "backtab".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::F(value) => format!("f{value}"),
        KeyCode::Char(value) => value.to_lowercase().to_string(),
        KeyCode::Null => "null".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::CapsLock => "capslock".to_string(),
        KeyCode::ScrollLock => "scrolllock".to_string(),
        KeyCode::NumLock => "numlock".to_string(),
        KeyCode::PrintScreen => "printscreen".to_string(),
        KeyCode::Pause => "pause".to_string(),
        KeyCode::Menu => "menu".to_string(),
        KeyCode::KeypadBegin => "keypadbegin".to_string(),
        KeyCode::Media(_) | KeyCode::Modifier(_) => String::new(),
    }
}

fn key_modifiers_from_crossterm(modifiers: CrosstermKeyModifiers) -> BTreeSet<KeyModifier> {
    let mut out = BTreeSet::new();
    if modifiers.contains(CrosstermKeyModifiers::CONTROL) {
        out.insert(KeyModifier::Control);
    }
    if modifiers.contains(CrosstermKeyModifiers::SHIFT) {
        out.insert(KeyModifier::Shift);
    }
    if modifiers.contains(CrosstermKeyModifiers::ALT) {
        out.insert(KeyModifier::Alt);
    }
    out
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

    pub fn set_regions_preserving(
        &mut self,
        regions: impl IntoIterator<Item = RegionId>,
        preferred: Option<&str>,
    ) {
        self.regions = regions.into_iter().collect();
        self.selected = preferred
            .and_then(|preferred| self.regions.iter().position(|region| region == preferred));
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

    #[test]
    fn focus_ring_preserves_region_when_frame_updates() {
        let mut ring = FocusRing::new(["a".to_string(), "b".to_string()]);
        ring.focus_next();

        ring.set_regions_preserving(
            ["c".to_string(), "b".to_string(), "d".to_string()],
            Some("b"),
        );

        assert_eq!(ring.current().map(String::as_str), Some("b"));
    }

    #[test]
    fn keymap_matches_crossterm_key_events() {
        let keymap = Keymap::default();

        assert!(keymap.matches_key_event(
            Action::CopySelection,
            &KeyEvent::new(
                KeyCode::Char('C'),
                CrosstermKeyModifiers::CONTROL | CrosstermKeyModifiers::SHIFT,
            ),
        ));
        assert!(!keymap.matches_key_event(
            Action::CopySelection,
            &KeyEvent::new(KeyCode::Char('c'), CrosstermKeyModifiers::NONE),
        ));
        assert!(keymap.matches_key_event(
            Action::FocusPreviousRegion,
            &KeyEvent::new(KeyCode::BackTab, CrosstermKeyModifiers::SHIFT),
        ));
        assert!(!keymap.matches_key_event(
            Action::CycleMode,
            &KeyEvent::new(KeyCode::BackTab, CrosstermKeyModifiers::SHIFT),
        ));
    }

    #[test]
    fn binding_display_labels_are_user_readable() {
        assert_eq!(
            KeyBinding::modified("c", [KeyModifier::Control, KeyModifier::Shift]).display_label(),
            "Ctrl+Shift+C"
        );
        assert_eq!(KeyBinding::plain("backtab").display_label(), "Shift+Tab");
    }
}
