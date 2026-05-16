use serde_json::Value;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ToolTimelineEntry {
    pub name: String,
    pub arguments: String,
}

impl ToolTimelineEntry {
    pub fn new(name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    pub fn running_message(&self) -> String {
        format!("tool_running: {}", self.label())
    }

    pub fn completed_message(&self, failed: bool) -> String {
        if failed {
            format!("tool_failed: {}", self.label())
        } else {
            format!("tool: {}", self.label())
        }
    }

    fn label(&self) -> String {
        let title = tool_title(&self.name);
        let arguments = argument_preview(&self.arguments);
        if arguments.is_empty() {
            title
        } else {
            format!("{title} {arguments}")
        }
    }
}

pub(super) fn fallback_entry(name: impl Into<String>) -> ToolTimelineEntry {
    ToolTimelineEntry::new(name, "")
}

fn tool_title(name: &str) -> String {
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

fn argument_preview(arguments: &str) -> String {
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
        Value::Object(map) => map
            .into_iter()
            .filter_map(|(key, value)| preview_json_field(&key, &value))
            .collect::<Vec<_>>()
            .join(" "),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_entry_formats_title_and_arguments() {
        let entry = ToolTimelineEntry::new(
            "grep",
            r##"{"pattern":"^#|name =|description","path":"README.md"}"##,
        );

        assert_eq!(
            entry.running_message(),
            r#"tool_running: Grep path: README.md pattern: "^#|name =|description""#
        );
        assert_eq!(
            entry.completed_message(false),
            r#"tool: Grep path: README.md pattern: "^#|name =|description""#
        );
    }

    #[test]
    fn failed_entry_uses_failed_prefix() {
        let entry = ToolTimelineEntry::new("read_file", r#"{"path":"missing.md"}"#);

        assert_eq!(
            entry.completed_message(true),
            "tool_failed: Read File path: missing.md"
        );
    }
}
