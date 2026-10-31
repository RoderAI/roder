//! What an action visibly did, as one line for the step's history.
//!
//! `page_changed` only says that the fingerprint moved; the effect says how:
//! the address it went to (and an HTTP status of 400 or more it landed on),
//! the controls whose label, value or state changed (at most four), and the
//! controls it showed or removed. It compares the on-screen actions of the
//! observations before and after, one entry per node, and a control whose
//! node was replaced (a redraw, a picker's copy of a field) is matched by
//! its role and label when exactly one earlier control had them. The rules
//! follow fastbrowse's effects.rs (MIT), reimplemented.
//!
//! An effect also says whether the action set something: the address, or a
//! control's label, value, checked or selected state, changed (a menu only
//! opening or closing does not count). A choice clicked (an option, a
//! checkbox, a radio) that set nothing did not take, and the stall rule
//! counts it as a step that changed nothing; a field that now shows the
//! choice counts as set, since an autocomplete pick can leave the typed
//! text exactly as it was. An action that set nothing names the fields its
//! form still needs (`blocking` in the snapshot: required and empty, or
//! marked invalid), since a refused submit says why only in the fields it
//! marks: its own form's fields when its control is in a form, every
//! blocking field on the page when it is not.
//!
//! The effect is recorded on the step and on [`crate::JevActionRecord`], and
//! the tool result shows it to the caller. It is not sent to the decision
//! model: that would change the request, and it
//! has not yet been measured against the hosted model (the eval variant
//! `effect` adds it for an A/B run).

use std::collections::HashMap;

use serde_json::Value;

/// How many changes, and how many shown or removed controls, are named.
const SHOWN: usize = 4;
/// How long a named value or label may be.
const TEXT_CHARS: usize = 60;

/// The roles whose click chooses a value, so it should leave some value,
/// selection or address changed.
const SETTING_ROLES: [&str; 6] = [
    "option",
    "checkbox",
    "radio",
    "switch",
    "menuitemradio",
    "menuitemcheckbox",
];

/// The summary of an action that changed nothing.
pub(crate) const NOTHING: &str = "nothing visible changed";

/// What an action did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Effect {
    pub(crate) summary: String,
    /// The address, or a control's label, value, checked or selected state,
    /// changed, or a field now shows the choice clicked.
    pub(crate) set_something: bool,
}

/// What the step knows beyond its two observations.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Landing<'a> {
    /// The main document's HTTP status, read when the action led to another
    /// document.
    pub(crate) status: Option<u16>,
    /// The choice the action clicked (see [`chooses`]), whose label a field
    /// may now show.
    pub(crate) chosen: Option<&'a Value>,
    /// The id of the form the action's control belongs to (the snapshot's
    /// `form.id`), if it does: only that form's fields are named as still
    /// needed. Without one, every blocking field on the page is.
    pub(crate) form: Option<&'a Value>,
}

/// Whether clicking `action` chooses a value: an option, checkbox, radio,
/// switch or checkable menu item.
pub(crate) fn chooses(action: &Value) -> bool {
    action["kind"] == "click"
        && action["role"]
            .as_str()
            .is_some_and(|role| SETTING_ROLES.contains(&role))
}

/// One control as the effect compares it.
#[derive(Debug, Clone, PartialEq)]
struct Control {
    node: i64,
    role: String,
    label: String,
    /// Label, value, checked, selected, expanded.
    state: [(&'static str, String); 5],
}

impl Control {
    fn of(action: &Value) -> Option<Self> {
        let node = action["node"].as_i64()?;
        let text = |key: &str| match &action[key] {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        // A select's actions read "Size → Large"; the control is "Size".
        let label = text("label")
            .split(" → ")
            .next()
            .unwrap_or_default()
            .to_string();
        let value = match (action["kind"].as_str(), action["filled"].as_bool()) {
            // A secret field's content is never read, only whether it holds any.
            (_, Some(filled)) => (if filled { "filled" } else { "empty" }).to_string(),
            (Some("select"), None) => text("current_value"),
            (Some("fill") | Some("click"), None) => text("value"),
            _ => String::new(),
        };
        Some(Self {
            node,
            role: text("role"),
            state: [
                ("label", squash(&label)),
                ("value", value),
                ("checked", text("checked")),
                ("selected", text("selected")),
                ("expanded", text("expanded")),
            ],
            label,
        })
    }

    /// Whether this field now shows `choice` (already [`plain`]), whole or
    /// as the start of it ("London" for "London, United Kingdom"): its
    /// value, or a combobox's label. A secret's "filled" shows nothing.
    fn shows(&self, choice: &str) -> bool {
        if SETTING_ROLES.contains(&self.role.as_str()) {
            return false;
        }
        let secret = matches!(self.state[1].1.as_str(), "filled" | "empty");
        let value = if secret {
            String::new()
        } else {
            plain(&self.state[1].1)
        };
        let label = match self.role.as_str() {
            "combobox" => plain(&self.label),
            _ => String::new(),
        };
        [value, label]
            .iter()
            .any(|shown| !shown.is_empty() && choice.contains(shown.as_str()))
    }
}

/// The on-screen controls of an observation, first action per node, in order.
fn controls(observation: &Value) -> Vec<Control> {
    let mut seen = Vec::new();
    let mut list = Vec::new();
    for action in observation["actions"].as_array().into_iter().flatten() {
        if action["offscreen"] == Value::Bool(true) || action["kind"] == "scroll" {
            continue;
        }
        let Some(control) = Control::of(action) else {
            continue;
        };
        if !seen.contains(&control.node) {
            seen.push(control.node);
            list.push(control);
        }
    }
    list
}

/// The fields marked `blocking`, first action per node, offscreen ones too:
/// a required field above the button just pressed is still needed. With
/// `form`, only that form's: a required field of another form (a footer's
/// newsletter box) is not why this one did nothing.
fn blocking(observation: &Value, form: Option<&Value>) -> Vec<Control> {
    let mut seen = Vec::new();
    let mut list = Vec::new();
    for action in observation["actions"].as_array().into_iter().flatten() {
        if action["blocking"] != Value::Bool(true)
            || form.is_some_and(|form| action["form"]["id"] != *form)
        {
            continue;
        }
        let Some(control) = Control::of(action) else {
            continue;
        };
        if !seen.contains(&control.node) {
            seen.push(control.node);
            list.push(control);
        }
    }
    list
}

/// What changed from `before` to `after`, or "nothing visible changed".
pub(crate) fn effect(before: &Value, after: &Value, landing: Landing<'_>) -> Effect {
    let old = controls(before);
    let new = controls(after);
    let mut parts = Vec::new();
    let navigated = before["url"] != after["url"];
    if navigated {
        parts.push(format!(
            "went to {}",
            short(after["url"].as_str().unwrap_or_default())
        ));
    }
    if after["opened_tab"] == Value::Bool(true) {
        parts.push("opened a new tab, now active".to_string());
    }
    if let Some(status) = landing.status.filter(|status| *status >= 400) {
        parts.push(format!(
            "HTTP {status}: destination did not load successfully"
        ));
    }
    // Node ids count from one in every document, so after a navigation or
    // on another tab the same id names an unrelated control: there, controls
    // are paired by role and label alone.
    let same = same_document(before, after);
    let by_node = old
        .iter()
        .filter(|_| same)
        .map(|control| (control.node, control))
        .collect::<HashMap<_, _>>();
    let mut by_name: HashMap<(&str, &str), Vec<&Control>> = HashMap::new();
    for control in &old {
        by_name
            .entry((control.role.as_str(), control.label.trim()))
            .or_default()
            .push(control);
    }
    let still = new
        .iter()
        .filter(|_| same)
        .map(|control| control.node)
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut setting = false;
    // Old and new nodes paired by role and label rather than by node.
    let mut paired = Vec::new();
    for now in &new {
        let was = by_node.get(&now.node).copied().or_else(|| {
            match by_name.get(&(now.role.as_str(), now.label.trim())) {
                Some(namesakes) if namesakes.len() == 1 && !still.contains(&namesakes[0].node) => {
                    Some(namesakes[0])
                }
                _ => None,
            }
        });
        let Some(was) = was else {
            continue;
        };
        if !same || was.node != now.node {
            paired.push((was.node, now.node));
        }
        for ((name, a), (_, b)) in was.state.iter().zip(&now.state) {
            if a != b {
                changes.push(format!(
                    "{} {name}: {} -> {}",
                    short(&was.label),
                    short(a),
                    short(b)
                ));
                // A menu opening or closing is not a value set.
                setting |= *name != "expanded";
            }
        }
    }
    if !changes.is_empty() {
        parts.push(format!("changed {}", listed_changes(&changes)));
    }
    let shown = new
        .iter()
        .filter(|control| {
            !by_node.contains_key(&control.node)
                && !paired.iter().any(|(_, new)| *new == control.node)
        })
        .collect::<Vec<_>>();
    let removed = old
        .iter()
        .filter(|control| {
            !still.contains(&control.node) && !paired.iter().any(|(old, _)| *old == control.node)
        })
        .collect::<Vec<_>>();
    if !shown.is_empty() {
        parts.push(format!("showed {}", listed(&shown)));
    }
    if !removed.is_empty() {
        parts.push(format!("removed {}", listed(&removed)));
    }
    if let Some(chosen) = landing.chosen.and_then(Control::of) {
        let choice = plain(&chosen.label);
        setting |= new.iter().any(|control| control.shows(&choice));
        // A menu that closed is visible, but the choice did not take.
        if !setting && !navigated && !parts.is_empty() {
            parts.push(format!("{} set no value", short(&chosen.label)));
        }
    }
    let mut summary = match parts.is_empty() {
        true => NOTHING.to_string(),
        false => parts.join("; "),
    };
    // A submit the form refuses moves nothing or reopens a picker, and says
    // why only in the fields it marks.
    if !(navigated || setting) {
        let needed = blocking(after, landing.form);
        if !needed.is_empty() {
            summary.push_str(&format!(
                "; fields the form still needs: {}",
                listed(&needed.iter().collect::<Vec<_>>())
            ));
        }
    }
    Effect {
        summary,
        set_something: navigated || setting,
    }
}

/// Whether both observations read the same document: the same
/// `performance.timeOrigin` (the page key's first entry) and no new tab. An
/// observation without a page key (a hosted browser's) is compared by URL.
pub(crate) fn same_document(before: &Value, after: &Value) -> bool {
    if after["opened_tab"] == Value::Bool(true) {
        return false;
    }
    match (before["page_key"].get(0), after["page_key"].get(0)) {
        (Some(was), Some(now)) => was == now,
        _ => before["url"] == after["url"],
    }
}

fn listed_changes(changes: &[String]) -> String {
    let mut text = changes[..changes.len().min(SHOWN)].join("; ");
    if changes.len() > SHOWN {
        text.push_str(&format!(" and {} more", changes.len() - SHOWN));
    }
    text
}

fn listed(controls: &[&Control]) -> String {
    let count = controls.len();
    let names = controls
        .iter()
        .take(SHOWN)
        .map(|control| short(&control.label))
        .collect::<Vec<_>>()
        .join(", ");
    let more = if count > SHOWN {
        format!(" and {} more", count - SHOWN)
    } else {
        String::new()
    };
    format!(
        "{count} control{}: {names}{more}",
        if count == 1 { "" } else { "s" }
    )
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Squashed and lower-cased, as a choice and what a field shows compare.
fn plain(text: &str) -> String {
    squash(text).to_lowercase()
}

fn short(text: &str) -> String {
    let text = squash(text);
    if text.is_empty() {
        return "empty".into();
    }
    if text.chars().count() <= TEXT_CHARS {
        return text;
    }
    let mut cut = text.chars().take(TEXT_CHARS - 1).collect::<String>();
    cut.push('…');
    cut
}

/// Bounded newly visible lines from already secret-scrubbed observations.
/// Unlike the control summary, this captures receipts and cart confirmations.
pub(crate) fn text_added(before: &Value, after: &Value) -> Vec<String> {
    let old = before["text"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .collect::<std::collections::HashSet<_>>();
    after["text"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !old.contains(line))
        .take(8)
        .map(|line| line.chars().take(160).collect())
        .collect()
}

#[cfg(test)]
mod tests;
