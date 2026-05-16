use std::collections::HashSet;

use serde_json::Value;

pub(super) fn tool_title(name: &str) -> String {
    let mut out = String::new();
    for part in name
        .split(['_', '-', '.'])
        .filter(|part| !part.trim().is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        "Tool".to_string()
    } else {
        out
    }
}

pub(super) fn tool_label(name: &str, arguments: &str) -> String {
    let title = tool_title(name);
    let arguments = argument_preview(arguments);
    if arguments.is_empty() {
        return title;
    }

    if let Some(primary) = arguments.strip_prefix("path: ") {
        return format!("{title}: {primary}");
    }

    format!("{title}: {arguments}")
}

pub(super) fn argument_preview(arguments: &str) -> String {
    let trimmed = arguments.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return String::new();
    }
    let preview = serde_json::from_str::<Value>(trimmed)
        .ok()
        .map(preview_json_value)
        .unwrap_or_else(|| trimmed.to_string());
    truncate(&preview.replace('\n', " "), 140)
}

fn preview_json_value(value: Value) -> String {
    match value {
        Value::Object(map) => {
            let priority = ["path", "query", "pattern", "start_line", "limit", "offset"];
            let mut fields = Vec::new();
            let mut used = HashSet::new();
            for key in priority {
                if let Some(value) = map.get(key)
                    && let Some(field) = preview_json_field(key, value)
                {
                    fields.push(field);
                    used.insert(key.to_string());
                }
            }
            for (key, value) in map {
                if used.contains(&key) {
                    continue;
                }
                if let Some(field) = preview_json_field(&key, &value) {
                    fields.push(field);
                }
            }
            fields.join(" ")
        }
        other => compact_json(&other),
    }
}

fn preview_json_field(key: &str, value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) if text.trim().is_empty() => None,
        Value::String(text) => Some(format!("{key}: {}", quote_if_needed(text))),
        Value::Array(values) if values.is_empty() => None,
        Value::Object(map) if map.is_empty() => None,
        other => Some(format!("{key}: {}", compact_json(other))),
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn quote_if_needed(value: &str) -> String {
    if value.contains(char::is_whitespace) {
        format!("{value:?}")
    } else {
        value.to_string()
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if out.len() < value.len() {
        out.push_str("...");
    }
    out
}
