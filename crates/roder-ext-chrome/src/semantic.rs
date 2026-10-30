//! Canonical semantic browser snapshots for Jev, Decisions and direct tools.
//! Rows carry hierarchy, role, label, state and optional backend-actionable refs.
//! Geometry belongs to each backend's diagnostic data, never this presentation.
use serde_json::{Value, json};
fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.into(),
    }
}
use crate::observed_cut::role_of;

/// Derive a conservative change set from complete observations. A new URL or
/// document, or either incomplete observation, requires full recovery.
pub fn changes(current: &Value, previous: Option<&Value>) -> Value {
    let rows = current["rows"].as_array().cloned().unwrap_or_default();
    let previous = previous.filter(|old| {
        old["url"] == current["url"]
            && old["document"] == current["document"]
            && old["limited"] != true
            && current["limited"] != true
    });
    let old = previous
        .and_then(|p| p["rows"].as_array())
        .cloned()
        .unwrap_or_default();
    let mut changed = Vec::new();
    for row in &rows {
        let before = old.iter().find(|p| p["id"] == row["id"]);
        if before != Some(row) {
            let mut change = row.clone();
            change["change"] = json!(if before.is_some() { "changed" } else { "added" });
            changed.push(change);
        }
    }
    let removed = old
        .iter()
        .filter(|p| !rows.iter().any(|r| r["id"] == p["id"]))
        .cloned()
        .collect::<Vec<_>>();
    let full = previous.is_none() || changed.len() + removed.len() > 100;
    let context = rows
        .iter()
        .filter(|row| changed.iter().any(|r| r["parent"] == row["id"]))
        .cloned()
        .collect::<Vec<_>>();
    let text = current["text"].as_str().unwrap_or_default();
    let displayed_text = if full {
        text.to_string()
    } else {
        text_delta(
            text,
            previous.unwrap()["text"].as_str().unwrap_or_default(),
            current["text_cut"] != true && previous.unwrap()["text_cut"] != true,
        )
    };
    json!({"mode":if full{"full"}else{"diff"},"rows":if full{rows}else{changed},"text":displayed_text,
        "removed":if full{Vec::new()}else{removed},"context":context,"limited":current["limited"]==true,
        "text_changed":full || previous.is_none_or(|p|p["text_hash"]!=current["text_hash"] || p["text_length"]!=current["text_length"]),
        "title_changed":previous.is_none_or(|p|p["title"]!=current["title"])})
}

fn text_delta(current: &str, previous: &str, complete: bool) -> String {
    fn excess(a: &str, b: &str, prefix: &str) -> Vec<String> {
        let mut remaining = std::collections::HashMap::<&str, usize>::new();
        for line in b.lines() {
            *remaining.entry(line).or_default() += 1;
        }
        a.lines()
            .filter_map(|line| match remaining.get_mut(line).filter(|n| **n > 0) {
                Some(n) => {
                    *n -= 1;
                    None
                }
                None => Some(format!("{prefix} {line}")),
            })
            .collect()
    }
    let mut lines = excess(current, previous, "+");
    if complete {
        lines.extend(excess(previous, current, "-"));
    }
    lines.join("\n")
}

/// Render a canonical snapshot or change set. A ref is used with the browser
/// tool family that supplied it; an internal planner resolves it to an offered
/// action. Passive structure never receives invented actionable references.
pub fn render(snapshot: &Value, line_limit: usize) -> String {
    let diff = snapshot["mode"] == "diff";
    let rows = snapshot["rows"].as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    lines.push(if diff {
        "Viewport changes (+ newly observed, ~ changed, - no longer observed; call look for full tree):"
    } else {
        "Snapshot (act by ref; indentation shows viewport hierarchy):"
    }.into());
    let mut parents = std::collections::HashMap::new();
    if let Some(context) = snapshot["context"].as_array() {
        for row in context {
            parents.insert(row["id"].as_str().unwrap_or_default(), row);
        }
    }
    for row in &rows {
        parents.insert(row["id"].as_str().unwrap_or_default(), row);
    }
    let row_room = line_limit.saturating_sub(lines.len() + 1);
    let mut output = Vec::new();
    for row in &rows {
        let mut parent = row["parent"].as_str();
        let mut depth = 0;
        while let Some(p) = parent.and_then(|key| parents.get(key)) {
            depth += 1;
            if depth >= 12 {
                break;
            }
            parent = p["parent"].as_str();
        }
        if diff {
            let prefix = if row["change"] == "added" { "+" } else { "~" };
            let context = row["parent"]
                .as_str()
                .and_then(|key| parents.get(key))
                .map(|p| format!(" (in {})", element_line(p)))
                .unwrap_or_default();
            output.push(format!("{prefix} {}{context}", element_line(row)));
        } else {
            output.push(format!("{}- {}", "  ".repeat(depth), element_line(row)));
        }
    }
    if let Some(removed) = snapshot["removed"].as_array() {
        output.extend(removed.iter().map(|row| format!("- {}", element_line(row))));
    }
    if diff && output.is_empty() {
        output.push("No semantic changes.".into());
    }
    lines.extend(output.iter().take(row_room).cloned());
    if snapshot["limited"] == true || output.len() > row_room {
        lines.push("… snapshot limited; unlisted nodes are not evidence of removal. Scroll or use a screenshot.".into());
    }
    lines.join("\n")
}

fn element_line(element: &Value) -> String {
    // The role is the page's own word: one line, cut.
    let mut line = format!(
        "{} {}",
        element["ref"].as_str().unwrap_or_default(),
        role_of([
            element["role"].as_str(),
            element["type"].as_str(),
            element["tag"].as_str(),
        ])
    );
    line = line.trim_start().to_string();
    let label = one_line(&element["label"], 80);
    if !label.is_empty() {
        line.push_str(&format!(" \"{label}\""));
    }
    if element["secret"] == json!(true) {
        line.push_str(match element["filled"] == json!(true) {
            true => " (secret field, filled)",
            false => " (secret field, empty)",
        });
    } else if let Some(value) = element["value"].as_str() {
        line.push_str(&format!(" = \"{}\"", one_line(&json!(value), 60)));
    }
    if let Some(checked) = element["checked"].as_bool() {
        line.push_str(if checked {
            " [checked]"
        } else {
            " [unchecked]"
        });
    }
    if let Some(expanded) = element["expanded"].as_bool() {
        line.push_str(if expanded {
            " [expanded]"
        } else {
            " [collapsed]"
        });
    }
    for state in ["selected", "busy"] {
        if let Some(value) = element[state].as_bool() {
            line.push_str(&format!(" [{state}={value}]"));
        }
    }
    if element["disabled"] == json!(true) {
        line.push_str(" [disabled]");
    }
    if element["draggable"] == json!(true) {
        line.push_str(" [draggable]");
    }
    if element["focused"] == json!(true) {
        line.push_str(" [focused]");
    }
    if let Some(level) = element["level"].as_u64() {
        line.push_str(&format!(" [level={level}]"));
    }
    if let Some(options) = element["options"].as_array() {
        let options = options
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | ");
        line.push_str(&format!(" options: {}", cut(&options, 160)));
    }
    if element["offscreen"] == json!(true) {
        line.push_str(" offscreen");
    }
    line
}

pub(crate) fn one_line(value: &Value, chars: usize) -> String {
    let text = value
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    cut(&text, chars)
}

#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;
