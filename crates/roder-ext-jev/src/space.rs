//! The indexed action space.
//!
//! A port of upstream Jev's `model.action_space`: one index per observed
//! element, each operation carrying its own valid targets, and everything that
//! is not an operation (scroll, wait) kept aside as a control. Ordering is part
//! of the contract — indices are assigned in observation order and the target
//! maps are read back in insertion order — so this uses `serde_json::Map`,
//! which preserves insertion order in this workspace.

use serde_json::{Map, Value, json};

/// Keys copied onto an element, in upstream's order.
const ELEMENT_KEYS: [&str; 5] = ["role", "value", "checked", "selected", "expanded"];

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

fn operation_for(kind: &str) -> Option<&'static str> {
    match kind {
        "click" => Some("CLICK"),
        "fill" => Some("TYPE_TEXT"),
        "select" => Some("SELECT"),
        _ => None,
    }
}

/// Build the action space from an observation's `actions`.
pub(crate) fn action_space(actions: &[Value]) -> ActionSpace {
    let mut space = ActionSpace::default();
    // Observed node id to its element index, in first-seen order.
    let mut indices: Vec<(Value, String)> = Vec::new();
    for action in actions {
        let kind = action["kind"].as_str().unwrap_or_default();
        let Some(operation) = operation_for(kind) else {
            let name = action["id"].as_str().unwrap_or_default().to_uppercase();
            space.controls.push((name, action.clone()));
            continue;
        };
        let node = action["node"].clone();
        if !indices.iter().any(|(known, _)| known == &node) {
            let index = (space.elements.len() + 1).to_string();
            indices.push((node.clone(), index.clone()));
            let mut element = Map::new();
            for key in ELEMENT_KEYS {
                if let Some(value) = action.get(key) {
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
            "element list must match upstream, including key order"
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
    fn an_empty_observation_has_no_operations() {
        let space = action_space(&[]);
        assert!(space.elements.is_empty());
        assert!(space.targets.is_empty());
        assert!(space.controls.is_empty());
    }
}
