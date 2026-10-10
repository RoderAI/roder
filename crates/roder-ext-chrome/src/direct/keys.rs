//! Key names as a person writes them ("Escape", "Shift+Tab", "Control+a")
//! turned into the DevTools key events a keyboard sends.

use anyhow::{bail, ensure};
use serde_json::{Value, json};

/// One key press: the modifiers held, and the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chord {
    /// `Input.dispatchKeyEvent`'s bit mask: Alt 1, Control 2, Meta 4, Shift 8.
    modifiers: u8,
    key: String,
    code: String,
    virtual_code: u32,
    /// What the key types, when it types something and no modifier other
    /// than Shift is held.
    text: Option<String>,
}

/// Named keys: DOM `key`, DOM `code`, Windows virtual key code, and what
/// they type.
const NAMED: &[(&str, &str, u32, Option<&str>)] = &[
    ("Control", "ControlLeft", 17, None),
    ("Alt", "AltLeft", 18, None),
    ("Shift", "ShiftLeft", 16, None),
    ("Meta", "MetaLeft", 91, None),
    ("Enter", "Enter", 13, Some("\r")),
    ("Escape", "Escape", 27, None),
    ("Tab", "Tab", 9, None),
    ("Backspace", "Backspace", 8, None),
    ("Delete", "Delete", 46, None),
    ("ArrowUp", "ArrowUp", 38, None),
    ("ArrowDown", "ArrowDown", 40, None),
    ("ArrowLeft", "ArrowLeft", 37, None),
    ("ArrowRight", "ArrowRight", 39, None),
    ("Home", "Home", 36, None),
    ("End", "End", 35, None),
    ("PageUp", "PageUp", 33, None),
    ("PageDown", "PageDown", 34, None),
    (" ", "Space", 32, Some(" ")),
    ("F1", "F1", 112, None),
    ("F2", "F2", 113, None),
    ("F3", "F3", 114, None),
    ("F4", "F4", 115, None),
    ("F6", "F6", 117, None),
    ("F7", "F7", 118, None),
    ("F8", "F8", 119, None),
    ("F9", "F9", 120, None),
    ("F10", "F10", 121, None),
    ("F11", "F11", 122, None),
    ("F12", "F12", 123, None),
    ("F5", "F5", 116, None),
];

/// Other spellings people use for the named keys.
fn alias(name: &str) -> &str {
    match name.to_ascii_lowercase().as_str() {
        "esc" => "Escape",
        "return" => "Enter",
        "space" | "spacebar" => " ",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "del" => "Delete",
        _ => name,
    }
}

fn modifier_bit(key: &str) -> u8 {
    match key {
        "Control" => 2,
        "Alt" => 1,
        "Meta" => 4,
        "Shift" => 8,
        _ => 0,
    }
}

impl Chord {
    /// Parse `Control+Shift+ArrowDown`, `Escape`, `a`, `Meta+A`.
    pub(crate) fn parse(raw: &str) -> anyhow::Result<Self> {
        // A bare space is the space bar.
        let raw = match raw.trim() {
            "" if !raw.is_empty() => "Space",
            trimmed => trimmed,
        };
        ensure!(
            !raw.is_empty(),
            "key must name a key, such as Escape or Enter"
        );
        // "+" alone, or a chord ending in "+", means the plus key.
        let (held, key) = match raw.strip_suffix("++") {
            Some(held) => (held, "+"),
            None if raw == "+" => ("", "+"),
            None => match raw.rsplit_once('+') {
                Some((held, key)) => (held, key),
                None => ("", raw),
            },
        };
        let mut modifiers = 0u8;
        for part in held.split('+').filter(|part| !part.is_empty()) {
            modifiers |= match part.trim().to_ascii_lowercase().as_str() {
                "alt" | "option" => 1,
                "control" | "ctrl" => 2,
                "meta" | "cmd" | "command" | "super" => 4,
                "shift" => 8,
                other => bail!("unknown modifier {other:?} in {raw:?}"),
            };
        }
        let key = alias(key.trim_start());
        if let Some((key, code, virtual_code, text)) = NAMED
            .iter()
            .find(|(name, ..)| name.eq_ignore_ascii_case(key))
        {
            return Ok(Self {
                modifiers,
                key: key.to_string(),
                code: code.to_string(),
                virtual_code: *virtual_code,
                text: text.filter(|_| modifiers & !8 == 0).map(str::to_string),
            });
        }
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            bail!(
                "unknown key {key:?}: use a single character or a name such as Escape, Enter, Tab or ArrowDown"
            );
        };
        let shifted = modifiers & 8 != 0;
        let typed = if shifted {
            c.to_uppercase().next().unwrap_or(c)
        } else {
            c
        };
        let (code, virtual_code) = match c.to_ascii_uppercase() {
            letter @ 'A'..='Z' => (format!("Key{letter}"), letter as u32),
            digit @ '0'..='9' => (format!("Digit{digit}"), digit as u32),
            _ => (String::new(), 0),
        };
        Ok(Self {
            modifiers,
            key: typed.to_string(),
            code,
            virtual_code,
            text: (modifiers & !8 == 0).then(|| typed.to_string()),
        })
    }

    /// The events to send, in order: key down (with its text), key up. No
    /// native key code: those are the platform's, and a Windows code sent as
    /// macOS's native one pressed another key (Escape's 27 opened Chrome's
    /// "About Chrome" page from a shown tab).
    pub(crate) fn events(&self) -> [Value; 2] {
        let mut down = json!({
            "type": if self.text.is_some() { "keyDown" } else { "rawKeyDown" },
            "modifiers": self.modifiers | modifier_bit(&self.key),
            "key": self.key,
            "code": self.code,
            "windowsVirtualKeyCode": self.virtual_code,
        });
        if let Some(text) = &self.text {
            down["text"] = json!(text);
            down["unmodifiedText"] = json!(text);
        }
        let up = json!({
            "type": "keyUp",
            "modifiers": self.modifiers,
            "key": self.key,
            "code": self.code,
            "windowsVirtualKeyCode": self.virtual_code,
        });
        [down, up]
    }

    /// Full physical chord: modifiers down, key down/up, modifiers up.
    pub(crate) fn press_events(&self) -> Vec<Value> {
        let mut events = Vec::new();
        let mut modifiers = 0;
        let mut releases = Vec::new();
        for (bit, key, code, virtual_code) in [
            (2, "Control", "ControlLeft", 17),
            (1, "Alt", "AltLeft", 18),
            (4, "Meta", "MetaLeft", 91),
            (8, "Shift", "ShiftLeft", 16),
        ] {
            if self.modifiers & bit == 0 {
                continue;
            }
            modifiers |= bit;
            events.push(json!({"type":"rawKeyDown","key":key,"code":code,
                "windowsVirtualKeyCode":virtual_code,"modifiers":modifiers}));
            releases.push((bit, key, code, virtual_code));
        }
        events.extend(self.events());
        for (bit, key, code, virtual_code) in releases.into_iter().rev() {
            modifiers &= !bit;
            events.push(json!({"type":"keyUp","key":key,"code":code,
                "windowsVirtualKeyCode":virtual_code,"modifiers":modifiers}));
        }
        events
    }

    /// macOS editing shortcuts need CDP editing commands in addition to the
    /// DOM modifier bits. Remote Linux/Windows tabs use their native defaults.
    pub(crate) fn editing_command(&self, mac: bool) -> Option<&'static str> {
        if !mac {
            return None;
        }
        match (self.modifiers, self.key.to_ascii_lowercase().as_str()) {
            (4, "a") => Some("selectAll"),
            (4, "c") => Some("copy"),
            (4, "v") => Some("paste"),
            (4, "x") => Some("cut"),
            (4, "z") => Some("undo"),
            (12, "z") => Some("redo"),
            _ => None,
        }
    }

    /// The Command chord that does what this Control chord does on Windows and
    /// Linux, for the editing family (select all, copy, paste, cut, undo,
    /// redo). A Mac page ignores these with Control held: only Command chords
    /// carry the editing commands, so a model that sends Control+a types over
    /// nothing it selected. `None` for every other chord.
    pub(crate) fn mac_equivalent(&self) -> Option<Self> {
        let key = self.key.to_ascii_lowercase();
        let chord = match (self.modifiers, key.as_str()) {
            (2, "a" | "c" | "v" | "x" | "z") => format!("Meta+{key}"),
            (10, "z") | (2, "y") => "Meta+Shift+z".to_string(),
            _ => return None,
        };
        Self::parse(&chord).ok()
    }

    /// The chord as a person writes it in prose: `Ctrl+Shift+Z`, `Cmd+A`.
    pub(crate) fn spoken(&self) -> String {
        let mut parts = Vec::new();
        for (bit, name) in [(2, "Ctrl"), (1, "Alt"), (4, "Cmd"), (8, "Shift")] {
            if self.modifiers & bit != 0 {
                parts.push(name.to_string());
            }
        }
        parts.push(match self.key.as_str() {
            " " => "Space".to_string(),
            key if key.chars().count() == 1 => key.to_uppercase(),
            key => key.to_string(),
        });
        parts.join("+")
    }

    /// Enter: what submits a form or presses a focused control.
    pub(crate) fn is_enter(&self) -> bool {
        self.key == "Enter"
    }

    /// Space with no modifier: what presses a focused button or box.
    pub(crate) fn is_space(&self) -> bool {
        self.key == " " && self.modifiers & !8 == 0
    }

    pub(crate) fn name(&self) -> String {
        let mut parts = Vec::new();
        for (bit, name) in [(2, "Control"), (1, "Alt"), (4, "Meta"), (8, "Shift")] {
            if self.modifiers & bit != 0 {
                parts.push(name.to_string());
            }
        }
        parts.push(match self.key.as_str() {
            " " => "Space".to_string(),
            key => key.to_string(),
        });
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_and_their_spellings() {
        let escape = Chord::parse("esc").unwrap();
        assert_eq!(escape.name(), "Escape");
        let [down, up] = escape.events();
        assert_eq!(down["type"], "rawKeyDown");
        assert_eq!(down["windowsVirtualKeyCode"], 27);
        assert!(down.get("text").is_none());
        assert_eq!(up["type"], "keyUp");
        let enter = Chord::parse("Return").unwrap();
        assert!(enter.is_enter());
        assert_eq!(enter.events()[0]["text"], "\r");
        assert_eq!(Chord::parse("Shift+Tab").unwrap().name(), "Shift+Tab");
        assert_eq!(Chord::parse("space").unwrap().events()[0]["text"], " ");
        assert!(Chord::parse(" ").unwrap().is_space());
        assert!(!Chord::parse("Control+Space").unwrap().is_space());
    }

    #[test]
    fn characters_and_modifiers() {
        let a = Chord::parse("a").unwrap();
        assert_eq!(a.events()[0]["text"], "a");
        assert_eq!(a.events()[0]["code"], "KeyA");
        let select_all = Chord::parse("Control+a").unwrap();
        assert_eq!(select_all.events()[0]["modifiers"], 2);
        // A shortcut types nothing.
        assert!(select_all.events()[0].get("text").is_none());
        let shifted = Chord::parse("Shift+a").unwrap();
        assert_eq!(shifted.events()[0]["text"], "A");
        assert_eq!(Chord::parse("Meta+Shift+z").unwrap().name(), "Meta+Shift+Z");
        assert_eq!(Chord::parse("+").unwrap().events()[0]["text"], "+");
        assert_eq!(Chord::parse("Control++").unwrap().name(), "Control++");
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("Hyper+a").is_err());
        assert!(Chord::parse("Launch").is_err());
    }
}

#[cfg(test)]
mod native_chord_tests {
    use super::*;

    #[test]
    fn control_editing_chords_have_a_command_twin_for_mac_pages() {
        let twin = |raw: &str| Chord::parse(raw).unwrap().mac_equivalent();
        for (control, command, editing) in [
            ("Control+a", "Meta+a", "selectAll"),
            ("Ctrl+c", "Meta+c", "copy"),
            ("Control+V", "Meta+v", "paste"),
            ("Control+x", "Meta+x", "cut"),
            ("Control+z", "Meta+z", "undo"),
            ("Control+Shift+z", "Meta+Shift+z", "redo"),
            ("Control+y", "Meta+Shift+z", "redo"),
        ] {
            let twin = twin(control).unwrap_or_else(|| panic!("{control} has a twin"));
            assert_eq!(
                twin.name(),
                Chord::parse(command).unwrap().name(),
                "{control}"
            );
            assert_eq!(twin.editing_command(true), Some(editing), "{control}");
        }
        // Everything else is the caller's chord, unchanged.
        for other in [
            "Meta+a",
            "a",
            "Control+b",
            "Control+Alt+a",
            "Control+Shift+a",
            "Control+Meta+a",
            "Shift+z",
            "Control+Enter",
            "Control+ArrowDown",
        ] {
            assert!(twin(other).is_none(), "{other} is not remapped");
        }
    }

    #[test]
    fn chords_are_spoken_the_way_a_person_names_them() {
        assert_eq!(Chord::parse("Control+a").unwrap().spoken(), "Ctrl+A");
        assert_eq!(
            Chord::parse("Control+y")
                .unwrap()
                .mac_equivalent()
                .unwrap()
                .spoken(),
            "Cmd+Shift+Z"
        );
        assert_eq!(Chord::parse("Enter").unwrap().spoken(), "Enter");
        assert_eq!(Chord::parse("Alt+Space").unwrap().spoken(), "Alt+Space");
    }
    #[test]
    fn modifier_down_and_up_bracket_the_key_and_mac_edit_command() {
        let chord = Chord::parse("Meta+a").unwrap();
        let events = chord.press_events();
        assert_eq!(
            events
                .iter()
                .map(|event| event["key"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Meta", "a", "a", "Meta"]
        );
        assert_eq!(events[0]["modifiers"], 4);
        assert_eq!(events[3]["modifiers"], 0);
        assert_eq!(chord.editing_command(true), Some("selectAll"));
        assert_eq!(chord.editing_command(false), None);
        assert_eq!(Chord::parse("Shift").unwrap().events()[0]["modifiers"], 8);
        assert_eq!(Chord::parse("Shift").unwrap().events()[1]["modifiers"], 0);
    }
}
