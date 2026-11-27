//! Compact, human-readable summaries of tool-call arguments for timeline rows.

use serde_json::{Map, Value};

/// Keys whose value is the "headline" of a call and is shown without a `key=` label.
const PRIMARY_KEYS: &[&str] = &[
    "command",
    "cmd",
    "path",
    "file_path",
    "query",
    "pattern",
    "url",
    "message",
    "prompt",
    "goal",
    "objective",
    "text",
    "input",
];

/// Width of the right-aligned duration column so timestamps line up.
pub(super) const RELATIVE_TIME_COLUMN_WIDTH: usize = 7; // "59m 59s"

pub(super) fn format_tool_arguments(arguments_json: &str) -> String {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return String::new();
    }
    match parse_object_lenient(trimmed) {
        Some(map) => summarize_object(&map),
        None if trimmed.starts_with('{') => String::new(),
        None => single_line(trimmed),
    }
}

/// Parses a JSON object, tolerating the truncated prefixes produced while tool
/// arguments are still streaming in.
fn parse_object_lenient(text: &str) -> Option<Map<String, Value>> {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) {
        return Some(map);
    }
    if !text.starts_with('{') {
        return None;
    }
    let mut repaired = text.trim_end().to_string();
    // A dangling escape would swallow the closing quote we add.
    if repaired.ends_with('\\') && !repaired.ends_with("\\\\") {
        repaired.pop();
    }
    for suffix in ["}", "\"}", "]}", "\"]}", "}}", "\"}}"] {
        if let Ok(Value::Object(map)) =
            serde_json::from_str::<Value>(&format!("{repaired}{suffix}"))
        {
            return Some(map);
        }
    }
    None
}

fn summarize_object(map: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    if let Some(Value::String(patch)) = map.get("patch").or_else(|| map.get("input"))
        && let Some(summary) = summarize_patch(patch)
    {
        return summary;
    }
    let mut primary_used = false;
    for key in PRIMARY_KEYS {
        if let Some(value) = map.get(*key)
            && let Some(text) = value_text(value)
        {
            // Only the first headline value stays unlabeled; later ones keep context.
            if primary_used {
                parts.push(format!("{key}={text}"));
            } else {
                parts.push(text);
                primary_used = true;
            }
        }
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for key in keys {
        if PRIMARY_KEYS.contains(&key.as_str()) {
            continue;
        }
        if let Some(text) = map.get(key).and_then(value_text) {
            parts.push(format!("{key}={text}"));
        }
    }
    parts.join("  ")
}

fn value_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let text = single_line(&text);
    (!text.is_empty()).then_some(text)
}

/// Collapses newlines and runs of whitespace so multi-line values fit one row.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Summarizes an `apply_patch` body as its file operations instead of raw markers.
fn summarize_patch(patch: &str) -> Option<String> {
    if !patch.contains("*** Begin Patch") {
        return None;
    }
    let ops: Vec<String> = patch
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            [
                ("*** Update File: ", "update"),
                ("*** Add File: ", "add"),
                ("*** Delete File: ", "delete"),
            ]
            .iter()
            .find_map(|(marker, verb)| {
                line.strip_prefix(marker)
                    .map(|path| format!("{verb} {}", path.trim()))
            })
        })
        .collect();
    Some(if ops.is_empty() {
        "patch".to_string()
    } else {
        ops.join(", ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_command_is_unlabeled() {
        assert_eq!(
            format_tool_arguments(r#"{"command":"ls -la","timeout_seconds":5}"#),
            "ls -la  timeout_seconds=5"
        );
    }

    #[test]
    fn streaming_partial_json_hides_raw_json() {
        assert_eq!(
            format_tool_arguments(r#"{"command":"sed -n '280,345p' src/reque"#),
            "sed -n '280,345p' src/reque"
        );
        assert_eq!(format_tool_arguments(r#"{"comm"#), "");
        assert_eq!(format_tool_arguments(r#"{"command":"a\"#), "a");
    }

    #[test]
    fn message_is_headline_without_label() {
        assert_eq!(
            format_tool_arguments(r#"{"target":"docs","message":"Agree raw map"}"#),
            "Agree raw map  target=docs"
        );
    }

    #[test]
    fn patch_shows_file_operations() {
        let args = serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** Add File: b.rs\n+z\n*** End Patch"
        })
        .to_string();
        assert_eq!(format_tool_arguments(&args), "update a.rs, add b.rs");
    }

    #[test]
    fn streaming_patch_shows_files_so_far() {
        assert_eq!(
            format_tool_arguments(
                r#"{"patch":"*** Begin Patch\n*** Update File: /tmp/x/crates/roder"#
            ),
            "update /tmp/x/crates/roder"
        );
    }
}
