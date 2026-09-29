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
            "modifiers": self.modifiers,
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
