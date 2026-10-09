//! The page as a tool result reads it, whichever browser it came from.
//!
//! The paired extension lists a page as a snapshot (`controls`, `text`, ...)
//! and Roder Desktop's browser as a look (`elements`, `text`, ...). Both are
//! read into an [`Observed`], and two of them are compared by [`compare`] into
//! the one sentence that leads an action's result: what the action changed.
//! The same page through either browser says the same sentence, because the
//! sentence is a function of what was observed, not of who observed it.
//!
//! Everything here comes from the page and is untrusted: a word of it is put
//! on one line, stripped of control characters, and cut before it is kept.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::observed_read::Read;

/// Characters of a control's label, value and selector a line carries.
const LABEL_CHARS: usize = 80;
const VALUE_CHARS: usize = 60;
const SELECTOR_CHARS: usize = 80;
/// Characters of an address and of a title in a sentence.
const URL_CHARS: usize = 110;
const TITLE_CHARS: usize = 80;
/// The most the extension keeps of a page's text, so a text this long was cut
/// by it.
const EXTENSION_TEXT_CHARS: usize = 12_000;

/// One thing on the page to act on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Control {
    pub(crate) reference: String,
    pub(crate) role: String,
    pub(crate) label: String,
    pub(crate) value: Option<String>,
    /// A secret field: its value is never read, only whether it holds one.
    pub(crate) secret: bool,
    pub(crate) filled: Option<bool>,
    pub(crate) checked: Option<bool>,
    pub(crate) expanded: Option<bool>,
    pub(crate) disabled: bool,
    /// For a `<select>`, which the extension can only be pointed at this way.
    pub(crate) selector: Option<String>,
}

impl Control {
    /// Whether anything about the control that a person would see differs.
    /// Where it is on the screen does not count, nor how it is addressed.
    fn differs(&self, other: &Self) -> bool {
        (
            &self.role,
            &self.label,
            &self.value,
            self.secret,
            self.filled,
        ) != (
            &other.role,
            &other.label,
            &other.value,
            other.secret,
            other.filled,
        ) || self.checked != other.checked
            || self.expanded != other.expanded
            || self.disabled != other.disabled
    }
}

/// A page, as one observation of it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Observed {
    pub(crate) url: String,
    pub(crate) title: String,
    /// The page's text on one line.
    pub(crate) text: String,
    /// The most text the observer keeps, when it keeps a fixed amount.
    pub(crate) text_limit: Option<usize>,
    pub(crate) controls: Vec<Control>,
    pub(crate) forms: Vec<String>,
    pub(crate) frames: Vec<String>,
    pub(crate) viewport: Option<(i64, i64)>,
    pub(crate) scroll_y: Option<i64>,
    /// The tab the observer says this is, when it says.
    pub(crate) tab_id: Option<i64>,
    /// The sections above that were read; the others are empty because they
    /// were not asked for, not because the page has none.
    pub(crate) read: Read,
}

impl Observed {
    /// An extension snapshot: `{title, url, text, controls, forms, iframes,
    /// viewport}`.
    pub(crate) fn from_snapshot(snapshot: &Value) -> Self {
        let controls = snapshot["controls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(extension_control)
            .collect();
        let forms = snapshot["forms"]
            .as_array()
            .into_iter()
            .flatten()
            .take(12)
            .map(form_line)
            .collect();
        let frames = snapshot["iframes"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .map(|frame| {
                let how = match frame["crossOrigin"] == true {
                    true => "cross-origin, not readable",
                    false => "same-origin",
                };
                let src = text_of(&frame["src"], 100);
                match src.is_empty() {
                    true => format!("iframe {} ({how})", frame["index"]),
                    false => format!("iframe {} ({how}) {src}", frame["index"]),
                }
            })
            .collect();
        let viewport = &snapshot["viewport"];
        Self {
            url: one_line(snapshot["url"].as_str().unwrap_or_default(), 300),
            title: one_line(snapshot["title"].as_str().unwrap_or_default(), 150),
            text: one_line(snapshot["text"].as_str().unwrap_or_default(), usize::MAX),
            text_limit: Some(EXTENSION_TEXT_CHARS),
            controls,
            forms,
            frames,
            viewport: viewport["width"]
                .as_f64()
                .zip(viewport["height"].as_f64())
                .map(|(width, height)| (width as i64, height as i64)),
            scroll_y: viewport["scrollY"].as_f64().map(|y| y as i64),
            tab_id: None,
            read: Read::default(),
        }
    }

    /// A look of Roder Desktop's browser: `{url, title, viewport, elements,
    /// text}`.
    pub(crate) fn from_look(look: &Value) -> Self {
        let viewport = &look["viewport"];
        Self {
            url: one_line(look["url"].as_str().unwrap_or_default(), 300),
            title: one_line(look["title"].as_str().unwrap_or_default(), 150),
            text: one_line(look["text"].as_str().unwrap_or_default(), usize::MAX),
            controls: look["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(look_control)
                .collect(),
            viewport: viewport["w"]
                .as_f64()
                .zip(viewport["h"].as_f64())
                .map(|(width, height)| (width as i64, height as i64)),
            scroll_y: viewport["scroll_y"].as_f64().map(|y| y as i64),
            ..Self::default()
        }
    }

    /// Whether the text is as long as the observer keeps, which it is only
    /// when the page had more. (A cut that lands on a space loses it to the
    /// trim: one character short.)
    pub(crate) fn text_cut_by_source(&self) -> bool {
        self.text_limit
            .is_some_and(|limit| self.text.chars().count() + 1 >= limit)
    }
}

fn extension_control(control: &Value) -> Option<Control> {
    let reference = reference(&control["ref"])?;
    let text = |key: &str| {
        control[key]
            .as_str()
            .map(|text| one_line(text, LABEL_CHARS))
            .filter(|text| !text.is_empty())
    };
    let tag = control["tag"].as_str().unwrap_or_default();
    let kind = control["type"].as_str().unwrap_or_default();
    let field = matches!(tag, "input" | "textarea" | "select");
    // A field's `text` is its value or its placeholder, and a select's is
    // all its options: its name is what labels it.
    let label = match field {
        true => text("ariaName")
            .or_else(|| text("ariaLabel"))
            .or_else(|| text("placeholder"))
            .or_else(|| text("name"))
            .or_else(|| text("id")),
        false => text("ariaName")
            .or_else(|| text("ariaLabel"))
            .or_else(|| text("text")),
    }
    .unwrap_or_default();
    let secret = kind == "password";
    // A checkbox's value is the word `on`, which says nothing.
    let value = control["value"]
        .as_str()
        .filter(|_| !secret && !matches!(kind, "checkbox" | "radio"))
        .map(|value| one_line(value, VALUE_CHARS))
        .filter(|value| !value.is_empty());
    Some(Control {
        reference,
        role: control["role"]
            .as_str()
            .filter(|role| !role.is_empty())
            .unwrap_or(tag)
            .to_string(),
        label,
        value,
        secret,
        filled: None,
        checked: control["checked"].as_bool(),
        expanded: None,
        disabled: control["disabled"] == true,
        selector: (tag == "select")
            .then(|| text_of(&control["selector"], SELECTOR_CHARS))
            .filter(|selector| !selector.is_empty()),
    })
}

fn look_control(element: &Value) -> Option<Control> {
    if element["kind"] == "graphic" {
        return None;
    }
    let secret = element["secret"] == true;
    Some(Control {
        reference: reference(&element["ref"])?,
        role: element["role"]
            .as_str()
            .or_else(|| element["type"].as_str())
            .or_else(|| element["tag"].as_str())
            .unwrap_or_default()
            .to_string(),
        label: text_of(&element["label"], LABEL_CHARS),
        value: element["value"]
            .as_str()
            .filter(|_| !secret)
            .map(|value| one_line(value, VALUE_CHARS))
            .filter(|value| !value.is_empty()),
        secret,
        filled: secret.then(|| element["filled"] == true),
        checked: element["checked"].as_bool(),
        expanded: element["expanded"].as_bool(),
        disabled: element["disabled"] == true,
        selector: None,
    })
}

fn form_line(form: &Value) -> String {
    let fields = form["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .take(10)
        .filter_map(|field| {
            let name = [&field["label"], &field["name"]]
                .into_iter()
                .map(|name| text_of(name, 40))
                .find(|name| !name.is_empty())
                .unwrap_or_default();
            Some(format!(
                "{} {name} ({}{})",
                reference(&field["ref"])?,
                text_of(&field["type"], 20),
                if field["required"] == true {
                    ", required"
                } else {
                    ""
                }
            ))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let name = [&form["name"], &form["id"]]
        .into_iter()
        .map(|name| text_of(name, 40))
        .find(|name| !name.is_empty())
        .unwrap_or_default();
    let mut line = [
        "form".to_string(),
        name,
        text_of(&form["method"], 10).to_uppercase(),
    ]
    .into_iter()
    .chain([text_of(&form["action"], 100)])
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    if !fields.is_empty() {
        line.push_str(&format!(": {fields}"));
    }
    line
}

/// A ref as it is written into a line: only the characters a ref has.
fn reference(value: &Value) -> Option<String> {
    let reference: String = value
        .as_str()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .take(48)
        .collect();
    (!reference.is_empty()).then_some(reference)
}

fn text_of(value: &Value, chars: usize) -> String {
    one_line(value.as_str().unwrap_or_default(), chars)
}

/// Page words on one line: no line breaks, control or direction-overriding
/// characters, runs of space as one, at most `chars` characters.
pub(crate) fn one_line(text: &str, chars: usize) -> String {
    let plain: String = text
        .chars()
        .map(|c| match c {
            c if c.is_control() => ' ',
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => ' ',
            '\u{feff}' => ' ',
            c => c,
        })
        .collect();
    clip(
        &plain.split_whitespace().collect::<Vec<_>>().join(" "),
        chars,
    )
}

/// `text` cut to `chars` characters, with an ellipsis where it was cut.
pub(crate) fn clip(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// What an action did to the page, in one sentence, and which controls it
/// touched.
#[derive(Debug, Clone, Default)]
pub(crate) struct Comparison {
    pub(crate) sentence: String,
    /// Whether there was an earlier observation to compare with.
    pub(crate) compared: bool,
    pub(crate) changed: bool,
    pub(crate) url_changed: bool,
    pub(crate) controls_changed: usize,
    pub(crate) text_changed: bool,
    /// Refs of controls that were not on the page before.
    pub(crate) added: HashSet<String>,
    /// Refs of controls that were and show something else now.
    pub(crate) altered: HashSet<String>,
}

/// Said when there is nothing to compare an observation with.
pub(crate) const NOTHING_TO_COMPARE: &str = "No earlier observation of this tab to compare with.";

/// Compare the page after an action with the page before it.
pub(crate) fn compare(before: Option<&Observed>, after: &Observed) -> Comparison {
    let Some(before) = before else {
        return Comparison {
            sentence: NOTHING_TO_COMPARE.to_string(),
            ..Comparison::default()
        };
    };
    let mut result = Comparison {
        compared: true,
        ..Comparison::default()
    };
    if before.url != after.url {
        result.url_changed = true;
        result.changed = true;
        let (from, to) = address_pair(&before.url, &after.url);
        result.sentence = format!("URL {from} -> {to}.");
        if before.title != after.title {
            result.sentence.push_str(&format!(
                " Title now \"{}\".",
                clip(&after.title, TITLE_CHARS)
            ));
        }
        return result;
    }
    // A section only one of the two pages read cannot be compared: the other's
    // emptiness is not the page's.
    let controls_known = before.read.controls && after.read.controls;
    let text_known = before.read.text && after.read.text;
    if controls_known {
        let earlier: HashMap<&str, &Control> = before
            .controls
            .iter()
            .map(|control| (control.reference.as_str(), control))
            .collect();
        let now: HashSet<&str> = after
            .controls
            .iter()
            .map(|control| control.reference.as_str())
            .collect();
        for control in &after.controls {
            match earlier.get(control.reference.as_str()) {
                None => {
                    result.added.insert(control.reference.clone());
                }
                Some(was) if was.differs(control) => {
                    result.altered.insert(control.reference.clone());
                }
                Some(_) => {}
            }
        }
        let removed = earlier.keys().filter(|key| !now.contains(**key)).count();
        result.controls_changed = result.added.len() + result.altered.len() + removed;
    }
    result.text_changed = text_known && before.text != after.text;
    let mut parts = Vec::new();
    match result.controls_changed {
        0 => {}
        1 => parts.push("1 control changed".to_string()),
        count => parts.push(format!("{count} controls changed")),
    }
    if result.text_changed {
        parts.push("page text changed".to_string());
    }
    if before.title != after.title {
        parts.push(format!("title now \"{}\"", clip(&after.title, TITLE_CHARS)));
    }
    if let (Some(was), Some(now)) = (before.scroll_y, after.scroll_y)
        && was != now
    {
        parts.push(format!("scrolled to y={now}"));
    }
    result.changed = !parts.is_empty();
    let unread: Vec<&str> = [(!controls_known, "controls"), (!text_known, "text")]
        .into_iter()
        .filter_map(|(missing, name)| missing.then_some(name))
        .collect();
    result.sentence = match (parts.is_empty(), unread.is_empty()) {
        (true, true) => "No visible change.".to_string(),
        (true, false) => format!(
            "No visible change in what was compared; {} not read both times.",
            unread.join(" and ")
        ),
        (false, _) => sentence(&parts.join("; ")),
    };
    result
}

/// `text` as a sentence: capital first, full stop last.
pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    format!("{}{}.", first.unwrap_or_default(), chars.as_str())
}

/// Two addresses, each cut to what a sentence has room for. When they only
/// differ beyond that, the end of each is shown instead.
fn address_pair(from: &str, to: &str) -> (String, String) {
    let (cut_from, cut_to) = (clip(from, URL_CHARS), clip(to, URL_CHARS));
    if cut_from != cut_to {
        return (cut_from, cut_to);
    }
    let tail = |url: &str| {
        let count = url.chars().count();
        let tail: String = url.chars().skip(count.saturating_sub(URL_CHARS)).collect();
        format!("…{tail}")
    };
    (tail(from), tail(to))
}

impl Comparison {
    /// Add what happened beside the page changing (a dialog answered). A
    /// page that did not change is not reported as if nothing happened.
    pub(crate) fn also(&mut self, event: &str) {
        self.sentence = if !self.compared {
            format!("{} {}", self.sentence, sentence(event))
        } else if self.changed {
            format!("{}; {event}.", self.sentence.trim_end_matches('.'))
        } else {
            format!(
                "{}; no other visible change.",
                sentence(event).trim_end_matches('.')
            )
        };
        self.changed = true;
    }

    /// The comparison as the result's data keeps it.
    pub(crate) fn data(&self) -> Value {
        serde_json::json!({
            "sentence": self.sentence,
            "compared": self.compared,
            "changed": self.changed,
            "url_changed": self.url_changed,
            "controls_changed": self.controls_changed,
            "text_changed": self.text_changed,
        })
    }
}

#[cfg(test)]
#[path = "observed_tests.rs"]
mod tests;
