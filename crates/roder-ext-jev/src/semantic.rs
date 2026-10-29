//! The same semantic row contract and formatter as direct browser tools.
use roder_ext_chrome::semantic::{changes, render};
use serde_json::{Value, json};

/// Format only after an owner's secret scrub, and retain a complete current
/// observation for freshness and full recovery. No publication happens here.
pub(crate) fn refresh(observation: &mut Value) {
    if !observation["snapshot"].is_object() {
        return;
    }
    loop {
        let view = changes(&observation["snapshot"], None);
        let text = render(&view, 100);
        if text.chars().count() <= 6000 {
            observation["dom_snapshot"] = json!(text);
            return;
        }
        let snapshot = &mut observation["snapshot"];
        snapshot["limited"] = json!(true);
        let Some(rows) = snapshot["rows"].as_array_mut() else {
            return;
        };
        if rows.pop().is_none() {
            return;
        }
    }
}

/// Each decision gets a change set relative to what its driver last read.
/// Private browser probes leave this baseline untouched. Backend action refs
/// remain associated with the complete offered action map.
pub(crate) fn for_model(observation: &Value, previous: Option<&Value>) -> Value {
    let mut view = observation.clone();
    if !observation["snapshot"].is_object() {
        return view;
    }
    let snapshot = changes(&observation["snapshot"], previous);
    view["dom_snapshot"] = json!(render(&snapshot, 100));
    view["text"] = snapshot["text"].clone();
    view["snapshot"] = snapshot;
    view
}
