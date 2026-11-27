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
    let scan = scan_prefix(text);
    // Close the prefix as-is, then fall back to dropping the trailing partial member
    // so complete fields still show while a later value is half-received.
    let mut candidates = Vec::with_capacity(2);
    let mut closed = text[..scan.end].to_string();
    if scan.in_string {
        closed.push('"');
    }
    closed.extend(scan.open.iter().rev().map(|open| match open {
        '[' => ']',
        _ => '}',
    }));
    candidates.push(closed);
    if let Some(comma) = scan.last_top_level_comma {
        candidates.push(format!("{}}}", &text[..comma]));
    }
    candidates.into_iter().find_map(
        |candidate| match serde_json::from_str::<Value>(&candidate) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        },
    )
}

struct PrefixScan {
    /// Byte length to keep; excludes a dangling escape backslash.
    end: usize,
    in_string: bool,
    open: Vec<char>,
    last_top_level_comma: Option<usize>,
}

fn scan_prefix(text: &str) -> PrefixScan {
    let mut in_string = false;
    // Byte index of the backslash that starts the escape currently being read.
    let mut escape_start: Option<usize> = None;
    let mut unicode_digits: Option<u8> = None;
    let mut open = Vec::new();
    let mut last_top_level_comma = None;
    for (index, ch) in text.char_indices() {
        if in_string {
            if let Some(digits) = unicode_digits {
                let digits = digits + 1;
                unicode_digits = (digits < 4).then_some(digits);
                if unicode_digits.is_none() {
                    escape_start = None;
                }
            } else if escape_start.is_some() {
                if ch == 'u' {
                    unicode_digits = Some(0);
                } else {
                    escape_start = None;
                }
            } else if ch == '\\' {
                escape_start = Some(index);
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => open.push(ch),
            '}' | ']' => {
                open.pop();
            }
            ',' if open.len() == 1 => last_top_level_comma = Some(index),
            _ => {}
        }
    }
    // Drop an escape (`\` or a partial `\uXX`) that the stream cut off.
    let end = escape_start.unwrap_or(text.len());
    PrefixScan {
        end,
        in_string,
        open,
        last_top_level_comma,
    }
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
    let mut ops: Vec<String> = Vec::new();
    for line in patch.lines().map(str::trim) {
        if let Some(dest) = line.strip_prefix("*** Move to: ") {
            if let Some(last) = ops.last_mut() {
                last.push_str(&format!(" -> {}", dest.trim()));
            }
            continue;
        }
        if let Some(op) = [
            ("*** Update File: ", "update"),
            ("*** Add File: ", "add"),
            ("*** Delete File: ", "delete"),
        ]
        .iter()
        .find_map(|(marker, verb)| {
            line.strip_prefix(marker)
                .map(|path| format!("{verb} {}", path.trim()))
        }) {
            ops.push(op);
        }
    }
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
    fn streaming_keeps_complete_fields_before_partial_value() {
        assert_eq!(
            format_tool_arguments(r#"{"command":"ls","timeout_seconds":"#),
            "ls"
        );
        assert_eq!(
            format_tool_arguments(r#"{"command":"ls","timeout_seconds":1"#),
            "ls  timeout_seconds=1"
        );
    }

    #[test]
    fn streaming_handles_backslash_runs() {
        // Escaped backslash followed by a dangling escape.
        assert_eq!(format_tool_arguments(r#"{"command":"a\\\"#), r"a\");
        assert_eq!(format_tool_arguments(r#"{"command":"a\\"#), r"a\");
    }

    #[test]
    fn streaming_drops_partial_unicode_escape() {
        assert_eq!(format_tool_arguments(r#"{"command":"caf\u00"#), "caf");
        assert_eq!(
            format_tool_arguments(r#"{"command":"caf\u00e9"#),
            "caf\u{e9}"
        );
    }

    #[test]
    fn patch_move_shows_destination() {
        let args = serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: a.rs\n*** Move to: b.rs\n@@\n-x\n+y\n*** End Patch"
        })
        .to_string();
        assert_eq!(format_tool_arguments(&args), "update a.rs -> b.rs");
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
