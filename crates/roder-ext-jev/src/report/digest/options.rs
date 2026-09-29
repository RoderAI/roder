//! The options Jev can act on, grouped by the card or section they sit in.

use std::collections::HashMap;

use serde_json::Value;

use super::{GROUP_LABELS, REPEATED_LINK, cut, one_line, text};

/// The options Jev can act on, on screen first, grouped by the card or
/// section they sit in (a twin's context, else the heading before it), in
/// the order the groups first appear.
pub(super) fn options(data: &Value, chars: usize, lines: usize) -> Vec<String> {
    let controls = data["controls"].as_array().cloned().unwrap_or_default();
    if controls.is_empty() || lines < 3 || chars < 80 {
        return Vec::new();
    }
    let total = controls.len();
    let mut link_labels: HashMap<String, usize> = HashMap::new();
    for control in &controls {
        if control["role"] == "link" {
            *link_labels
                .entry(text(&control["label"]).to_string())
                .or_default() += 1;
        }
    }
    let mut ordered = controls
        .iter()
        .filter(|control| {
            control["role"] != "link"
                || link_labels
                    .get(text(&control["label"]))
                    .is_none_or(|count| *count <= REPEATED_LINK)
        })
        .collect::<Vec<_>>();
    // On screen first, each part in observed order.
    ordered.sort_by_key(|control| control["offscreen"] == Value::Bool(true));
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for control in ordered {
        // A twin's context, else the heading it sits under.
        let group = control["context"]
            .as_str()
            .or_else(|| control["section"].as_str())
            .map(|context| cut(&one_line(context), 60))
            .unwrap_or_else(|| "(page)".into());
        let label = option(control);
        if label.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(name, _)| *name == group) {
            Some((_, labels)) => labels.push(label),
            None => groups.push((group, vec![label])),
        }
    }
    let mut body = Vec::new();
    let mut shown = 0;
    let mut used = 0;
    // The heading and the closing marker line.
    let room_lines = lines - 2;
    let room_chars = chars.saturating_sub(120);
    for (group, labels) in &groups {
        for chunk in labels.chunks(GROUP_LABELS) {
            let line = format!("  {group}: {}", chunk.join(" · "));
            let cost = line.chars().count() + 1;
            if body.len() >= room_lines || used + cost > room_chars {
                break;
            }
            used += cost;
            shown += chunk.len();
            body.push(line);
        }
    }
    let mut out = vec![format!(
        "Options Jev can act on ({shown} of {total} shown, grouped by where they sit):"
    )];
    out.extend(body);
    if shown < total {
        out.push(format!(
            "  … ({} more controls, repeated navigation links included)",
            total - shown
        ));
    }
    out
}

/// One option as the list shows it: its label, and a field's or choice's
/// value.
fn option(control: &Value) -> String {
    let label = cut(&one_line(text(&control["label"])), 60);
    let value = control["value"]
        .as_str()
        .map(|value| cut(&one_line(value), 40));
    match (text(&control["kind"]), value) {
        ("fill", Some(value)) => format!("{label} [field: \"{value}\"]"),
        ("fill", None) => format!("{label} [field]"),
        ("select", Some(value)) => format!("{label} [choice: {value}]"),
        ("select", None) => format!("{label} [choice]"),
        (_, Some(value)) if value == "checked" => format!("{label} [checked]"),
        _ => label,
    }
}
