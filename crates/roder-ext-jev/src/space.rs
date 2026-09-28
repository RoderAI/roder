//! The indexed action space.
//!
//! A port of upstream Jev's `model.action_space`: one index per observed
//! element, each operation carrying its own valid targets, and everything that
//! is not an operation (scroll, wait) kept aside as a control. Ordering is part
//! of the contract — indices are assigned in observation order and the target
//! maps are read back in insertion order — so this uses `serde_json::Map`,
//! which preserves insertion order in this workspace.

use serde_json::{Map, Value, json};

/// Keys copied onto an element, in upstream's order, with Jev's own
/// `input_type` (a text field's kind: `text`, `email`, `textarea`,
/// `password`, `one-time-code`...), `filled` (present only on a password or
/// one-time-code field, whose `value` is never read: whether it holds
/// anything), `scrolled` (where a box that scrolls inside itself is: `top`,
/// `partway` or `bottom`) and `offscreen` (present only on a control outside
/// the viewport).
const ELEMENT_KEYS: [&str; 9] = [
    "role",
    "input_type",
    "value",
    "filled",
    "checked",
    "selected",
    "expanded",
    "scrolled",
    "offscreen",
];

/// A native date or time input's `input_type` stays out of the decision
/// request: told a field is a `date`, the model clicks it to open a picker
/// that never shows in the page (the live `date_field` task stalled). The
/// text helper is told it, with its ISO format.
const DATE_TYPES: [&str; 5] = ["date", "datetime-local", "month", "week", "time"];

/// An action's value for `key` as the decision model is shown it.
pub(crate) fn shown<'a>(action: &'a Value, key: &str) -> Option<&'a Value> {
    let value = action.get(key)?;
    if key == "input_type"
        && value
            .as_str()
            .is_some_and(|kind| DATE_TYPES.contains(&kind))
    {
        return None;
    }
    Some(value)
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ActionSpace {
    /// Elements in index order; index 1 is `elements[0]`.
    pub(crate) elements: Vec<Value>,
    /// Operation (`CLICK`, `TYPE_TEXT`, `SELECT`) to its targets, each target
    /// pointing at the observed action it executes.
    pub(crate) targets: Vec<(String, Vec<(String, Value)>)>,
    /// Non-operation controls such as `SCROLL_DOWN` and `WAIT`, upper-cased.
    pub(crate) controls: Vec<(String, Value)>,
}

impl ActionSpace {
    pub(crate) fn targets_for(&self, operation: &str) -> Option<&Vec<(String, Value)>> {
        self.targets
            .iter()
            .find(|(name, _)| name == operation)
            .map(|(_, group)| group)
    }

    pub(crate) fn control(&self, name: &str) -> Option<&Value> {
        self.controls
            .iter()
            .find(|(control, _)| control == name)
            .map(|(_, action)| action)
    }

    pub(crate) fn target_action(&self, operation: &str, target: &str) -> Option<&Value> {
        self.targets_for(operation)?
            .iter()
            .find(|(index, _)| index == target)
            .map(|(_, action)| action)
    }
}

fn operation_for(action: &Value) -> Option<&'static str> {
    match action["kind"].as_str().unwrap_or_default() {
        "click" => Some("CLICK"),
        "fill" => Some("TYPE_TEXT"),
        "select" => Some("SELECT"),
        // Jev's own: Enter in the focused text field.
        "enter" => Some("PRESS_ENTER"),
        // Jev's own: scrolling a box that scrolls inside itself. The window's
        // scroll has no node and stays a control.
        "scroll" if action.get("node").is_some() => Some(match action["direction"].as_str() {
            Some("up") => "SCROLL_REGION_UP",
            _ => "SCROLL_REGION_DOWN",
        }),
        _ => None,
    }
}

/// TypeSafe rejects a choice with more options than this, so no operation
/// may offer more targets. The snapshot's 250-action cap stays below it today.
pub(crate) const MAX_TARGETS: usize = 255;

/// Build the action space from an observation's `actions`.
pub(crate) fn action_space(actions: &[Value]) -> ActionSpace {
    let mut space = ActionSpace::default();
    // Observed node id to its element index, in first-seen order.
    let mut indices: Vec<(Value, String)> = Vec::new();
    for action in actions {
        let kind = action["kind"].as_str().unwrap_or_default();
        let Some(operation) = operation_for(action) else {
            let name = action["id"].as_str().unwrap_or_default().to_uppercase();
            space.controls.push((name, action.clone()));
            continue;
        };
        let full = space
            .targets_for(operation)
            .is_some_and(|group| group.len() >= MAX_TARGETS);
        if full {
            continue;
        }
        let node = action["node"].clone();
        if !indices.iter().any(|(known, _)| known == &node) {
            let index = (space.elements.len() + 1).to_string();
            indices.push((node.clone(), index.clone()));
            let mut element = Map::new();
            for key in ELEMENT_KEYS {
                if let Some(value) = shown(action, key) {
                    element.insert(key.into(), value.clone());
                }
            }
            element.insert("index".into(), json!(index));
            let label = action["label"].as_str().unwrap_or_default();
            // A select's label carries " → option"; the element keeps the field.
            let label = label.split(" → ").next().unwrap_or(label);
            element.insert("label".into(), json!(label));
            element.insert("operations".into(), json!([]));
            if kind == "select" {
                element.insert(
                    "value".into(),
                    action
                        .get("current_value")
                        .cloned()
                        .unwrap_or_else(|| json!("")),
                );
                element.insert("options".into(), json!([]));
            }
            // Jev's own: what tells this element apart from others that read
            // the same. Absent unless the snapshot found one.
            if let Some(context) = action.get("context") {
                element.insert("context".into(), context.clone());
            }
            space.elements.push(Value::Object(element));
        }
        let index = indices
            .iter()
            .find(|(known, _)| known == &node)
            .map(|(_, index)| index.clone())
            .expect("index recorded above");
        let position: usize = index.parse::<usize>().expect("numeric index") - 1;
        let element = &mut space.elements[position];
        let operations = element["operations"]
            .as_array_mut()
            .expect("operations array");
        if !operations.iter().any(|known| known == operation) {
            operations.push(json!(operation));
        }
        let mut target = index.clone();
        if kind == "select" {
            let options = element["options"].as_array_mut().expect("options array");
            target = format!("{index}:{}", options.len() + 1);
            options.push(json!({
                "index": target,
                "label": action["label"],
                "value": action["value"],
            }));
        }
        match space.targets.iter_mut().find(|(name, _)| name == operation) {
            Some((_, group)) => group.push((target, action.clone())),
            None => space
                .targets
                .push((operation.to_string(), vec![(target, action.clone())])),
        }
    }
    space
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/action_space.json")).unwrap()
    }

    #[test]
    fn matches_upstream_elements_exactly() {
        let fixture = fixture();
        let actions = fixture["input_actions"].as_array().unwrap();
        let space = action_space(actions);
        assert_eq!(
            Value::Array(space.elements.clone()),
            fixture["elements"],
            "element list must match the recorded fixture (upstream's plus Jev's offscreen control), including key order"
        );
        // Key order is part of what the model reads; compare serialized form.
        for (ours, upstream) in space
            .elements
            .iter()
            .zip(fixture["elements"].as_array().unwrap())
        {
            assert_eq!(ours.to_string(), upstream.to_string());
        }
    }

    #[test]
    fn matches_upstream_target_grouping_and_order() {
        let fixture = fixture();
        let space = action_space(fixture["input_actions"].as_array().unwrap());
        let order = fixture["target_order"].as_object().unwrap();
        assert_eq!(space.targets.len(), order.len());
        for (operation, expected) in order {
            let group = space
                .targets_for(operation)
                .unwrap_or_else(|| panic!("missing operation {operation}"));
            let ours = group
                .iter()
                .map(|(index, _)| Value::String(index.clone()))
                .collect::<Vec<_>>();
            assert_eq!(&Value::Array(ours), expected, "targets for {operation}");
        }
        // Each target maps to the same observed action upstream chose.
        for (operation, expected) in fixture["targets"].as_object().unwrap() {
            for (target, id) in expected.as_object().unwrap() {
                assert_eq!(
                    space.target_action(operation, target).map(|a| &a["id"]),
                    Some(id)
                );
            }
        }
    }

    #[test]
    fn matches_upstream_controls() {
        let fixture = fixture();
        let space = action_space(fixture["input_actions"].as_array().unwrap());
        let expected = fixture["controls"].as_object().unwrap();
        assert_eq!(space.controls.len(), expected.len());
        for (name, id) in expected {
            assert_eq!(space.control(name).map(|action| &action["id"]), Some(id));
        }
    }

    #[test]
    fn twins_carry_their_context_after_the_existing_keys() {
        let actions = [
            json!({"id": "e1", "node": 1, "role": "button", "kind": "click", "value": "",
                   "label": "Add to cart", "context": "Trail Runner"}),
            json!({"id": "e2", "node": 2, "role": "combobox", "kind": "select", "value": "m",
                   "current_value": "S", "label": "Size → M", "context": "Trail Runner"}),
            json!({"id": "e3", "node": 3, "role": "button", "kind": "click", "value": "",
                   "label": "Plain"}),
        ];
        let space = action_space(&actions);
        assert_eq!(
            space.elements[0].to_string(),
            r#"{"role":"button","value":"","index":"1","label":"Add to cart","operations":["CLICK"],"context":"Trail Runner"}"#
        );
        let select = space.elements[1].as_object().unwrap();
        assert_eq!(
            select.keys().collect::<Vec<_>>(),
            [
                "role",
                "value",
                "index",
                "label",
                "operations",
                "options",
                "context"
            ]
        );
        assert!(space.elements[2].get("context").is_none());
    }

    #[test]
    fn a_text_field_says_what_kind_it_is_after_its_role() {
        let actions = [
            json!({"id": "e1", "node": 1, "role": "textbox", "kind": "fill",
            "value": "", "label": "Note", "input_type": "textarea"}),
        ];
        let space = action_space(&actions);
        assert_eq!(
            space.elements[0].to_string(),
            r#"{"role":"textbox","input_type":"textarea","value":"","index":"1","label":"Note","operations":["TYPE_TEXT"]}"#
        );
        // A native date input's type is for the text helper only.
        let date = [
            json!({"id": "e1", "node": 1, "role": "textbox", "kind": "fill",
            "value": "", "label": "Departure", "input_type": "date"}),
        ];
        assert!(action_space(&date).elements[0].get("input_type").is_none());
    }

    #[test]
    fn a_secret_field_says_its_kind_and_whether_it_is_filled_never_its_value() {
        let actions = [
            json!({"id": "e1", "node": 1, "role": "textbox", "kind": "fill", "value": "",
                   "label": "Password", "input_type": "password", "filled": true}),
            json!({"id": "e2", "node": 2, "role": "textbox", "kind": "fill", "value": "",
                   "label": "Code", "input_type": "one-time-code", "filled": false}),
        ];
        let space = action_space(&actions);
        assert_eq!(
            space.elements[0].to_string(),
            r#"{"role":"textbox","input_type":"password","value":"","filled":true,"index":"1","label":"Password","operations":["TYPE_TEXT"]}"#
        );
        assert_eq!(space.elements[1]["input_type"], json!("one-time-code"));
        assert_eq!(space.elements[1]["filled"], json!(false));
    }

    #[test]
    fn no_operation_offers_more_targets_than_a_choice_accepts() {
        let actions = (1..=300)
            .map(|n| {
                json!({"id": format!("e{n}"), "node": n, "role": "button", "kind": "click",
                            "value": "", "label": format!("Cell {n}")})
            })
            .collect::<Vec<_>>();
        let space = action_space(&actions);
        assert_eq!(space.targets_for("CLICK").unwrap().len(), MAX_TARGETS);
        assert_eq!(space.elements.len(), MAX_TARGETS);
        assert_eq!(space.elements.last().unwrap()["label"], json!("Cell 255"));
    }

    #[test]
    fn an_empty_observation_has_no_operations() {
        let space = action_space(&[]);
        assert!(space.elements.is_empty());
        assert!(space.targets.is_empty());
        assert!(space.controls.is_empty());
    }
}
