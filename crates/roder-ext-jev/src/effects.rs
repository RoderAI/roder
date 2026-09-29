//! What an action visibly did, as one line for the step's history.
//!
//! `page_changed` only says that the fingerprint moved; the effect says how:
//! the address it went to, the controls whose label, value or state changed
//! (at most four), and the controls it showed or removed. It compares the
//! on-screen actions of the observations before and after, one entry per
//! node, and a control whose node was replaced (a redraw, a picker's copy of
//! a field) is matched by its role and label when exactly one earlier control
//! had them. The rules follow fastbrowse's effects.py (MIT), reimplemented.
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

/// What changed from `before` to `after`, or "nothing visible changed".
pub(crate) fn effect(before: &Value, after: &Value) -> String {
    let old = controls(before);
    let new = controls(after);
    let mut parts = Vec::new();
    if before["url"] != after["url"] {
        parts.push(format!(
            "went to {}",
            short(after["url"].as_str().unwrap_or_default())
        ));
    }
    if after["opened_tab"] == Value::Bool(true) {
        parts.push("opened a new tab, now active".to_string());
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
    if parts.is_empty() {
        return "nothing visible changed".into();
    }
    parts.join("; ")
}

/// Whether both observations read the same document: the same
/// `performance.timeOrigin` (the page key's first entry) and no new tab. An
/// observation without a page key (a hosted browser's) is compared by URL.
fn same_document(before: &Value, after: &Value) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(url: &str, actions: Value) -> Value {
        json!({"url": url, "actions": actions})
    }

    #[test]
    fn nothing_changed_says_so() {
        let before = page(
            "https://a.test/",
            json!([
                {"node": 1, "kind": "click", "role": "button", "label": "Retry", "value": ""},
            ]),
        );
        assert_eq!(effect(&before, &before.clone()), "nothing visible changed");
    }

    #[test]
    fn a_navigation_and_shown_and_removed_controls_are_named() {
        let before = page(
            "https://a.test/list",
            json!([
                {"node": 1, "kind": "click", "role": "link", "label": "Open report", "value": ""},
                {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
            ]),
        );
        let after = page(
            "https://a.test/report",
            json!([
                {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
                {"node": 3, "kind": "click", "role": "button", "label": "Approve", "value": ""},
                {"node": 4, "kind": "click", "role": "button", "label": "Reject", "value": ""},
                // Offscreen controls and scroll boxes are not news.
                {"node": 5, "kind": "click", "role": "button", "label": "Far", "offscreen": true},
            ]),
        );
        assert_eq!(
            effect(&before, &after),
            "went to https://a.test/report; showed 2 controls: Approve, Reject; \
             removed 1 control: Open report"
        );
    }

    #[test]
    fn value_and_state_changes_are_listed_up_to_four() {
        let before = page(
            "https://a.test/",
            json!([
                {"node": 1, "kind": "fill", "role": "textbox", "label": "Name", "value": ""},
                {"node": 1, "kind": "click", "role": "textbox", "label": "Open Name", "value": ""},
                {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Small",
                 "current_value": "Small"},
                {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Large",
                 "current_value": "Small"},
                {"node": 3, "kind": "click", "role": "checkbox", "label": "Gift", "checked": "false"},
                {"node": 4, "kind": "click", "role": "button", "label": "Menu", "expanded": "false"},
                {"node": 5, "kind": "click", "role": "button", "label": "Count 1", "value": ""},
            ]),
        );
        let after = page(
            "https://a.test/",
            json!([
                {"node": 1, "kind": "fill", "role": "textbox", "label": "Name", "value": "Ada"},
                {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Small",
                 "current_value": "Large"},
                {"node": 3, "kind": "click", "role": "checkbox", "label": "Gift", "checked": "true"},
                {"node": 4, "kind": "click", "role": "button", "label": "Menu", "expanded": "true"},
                {"node": 5, "kind": "click", "role": "button", "label": "Count 2", "value": ""},
            ]),
        );
        assert_eq!(
            effect(&before, &after),
            "changed Name value: empty -> Ada; Size value: Small -> Large; \
             Gift checked: false -> true; Menu expanded: false -> true and 1 more"
        );
    }

    #[test]
    fn a_secret_field_is_reported_filled_never_by_its_value() {
        let field = |filled: bool| {
            page(
                "https://a.test/",
                json!([{"node": 1, "kind": "fill", "role": "textbox", "label": "Password",
                        "value": "", "input_type": "password", "filled": filled}]),
            )
        };
        assert_eq!(
            effect(&field(false), &field(true)),
            "changed Password value: empty -> filled"
        );
    }

    #[test]
    fn a_replaced_control_is_matched_by_its_role_and_label() {
        let before = page(
            "https://a.test/",
            json!([
                {"node": 1, "kind": "fill", "role": "combobox", "label": "From", "value": "Lon"},
            ]),
        );
        let after = page(
            "https://a.test/",
            json!([
                {"node": 9, "kind": "fill", "role": "combobox", "label": "From", "value": "London"},
            ]),
        );
        assert_eq!(effect(&before, &after), "changed From value: Lon -> London");
    }

    /// After a navigation, node 1 is another document's first control: it is
    /// not the old node 1 with a new label.
    #[test]
    fn node_ids_are_not_paired_across_documents() {
        let mut before = page(
            "https://a.test/cart",
            json!([
                {"node": 1, "kind": "click", "role": "button", "label": "Checkout", "value": ""},
                {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
            ]),
        );
        before["page_key"] = json!([1000.5, "https://a.test/cart"]);
        let mut after = page(
            "https://a.test/pay",
            json!([
                {"node": 1, "kind": "fill", "role": "textbox", "label": "Card", "value": ""},
                {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": "x"},
            ]),
        );
        after["page_key"] = json!([2000.25, "https://a.test/pay"]);
        assert_eq!(
            effect(&before, &after),
            "went to https://a.test/pay; changed Help value: empty -> x; \
             showed 1 control: Card; removed 1 control: Checkout"
        );
        // A tab the action opened is another document too.
        let tabbed = page(
            "https://a.test/cart",
            json!([
                {"node": 1, "kind": "click", "role": "button", "label": "Print", "value": ""},
            ]),
        );
        let mut opened = tabbed.clone();
        opened["opened_tab"] = json!(true);
        opened["actions"][0]["label"] = json!("Close");
        assert_eq!(
            effect(&tabbed, &opened),
            "opened a new tab, now active; showed 1 control: Close; removed 1 control: Print"
        );
        // The same document keeps pairing by node.
        let mut same = before.clone();
        same["actions"][0]["label"] = json!("Checkout (2)");
        assert_eq!(
            effect(&before, &same),
            "changed Checkout label: Checkout -> Checkout (2)"
        );
    }

    #[test]
    fn a_new_tab_is_named() {
        let before = page("https://a.test/list", json!([]));
        let mut after = page("https://a.test/report", json!([]));
        after["opened_tab"] = json!(true);
        assert_eq!(
            effect(&before, &after),
            "went to https://a.test/report; opened a new tab, now active"
        );
    }
}
