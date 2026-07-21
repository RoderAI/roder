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

/// One child row parsed from an `<agent_swarm_result>` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentSwarmChildRow {
    pub outcome: String,
    pub item: Option<String>,
    pub agent_id: Option<String>,
}

impl AgentSwarmChildRow {
    /// Status glyph for the child outcome.
    pub fn glyph(&self) -> &'static str {
        match self.outcome.as_str() {
            "completed" => "✓",
            "failed" => "✗",
            "aborted" => "⊘",
            _ => "•",
        }
    }

    /// Compact one-line label, e.g. `✓ README.md  (5dc6a1a5)`.
    pub fn label(&self) -> String {
        let mut out = format!("{} ", self.glyph());
        match &self.item {
            Some(item) => out.push_str(&truncate(item, 60)),
            None => out.push_str("(resume)"),
        }
        if let Some(agent_id) = &self.agent_id {
            out.push_str(&format!("  ({})", short_agent_id(agent_id)));
        }
        out
    }
}

/// A parsed, display-ready view of an `<agent_swarm_result>` tool output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentSwarmResultSummary {
    /// e.g. `completed: 2, failed: 1`.
    pub summary: String,
    pub children: Vec<AgentSwarmChildRow>,
}

/// Parse the deterministic `<agent_swarm_result>` text emitted by the
/// `agent_swarm` tool into compact display rows. Returns `None` when the text
/// is not a swarm result block.
pub(super) fn agent_swarm_result_summary(output: &str) -> Option<AgentSwarmResultSummary> {
    if !output.contains("<agent_swarm_result>") {
        return None;
    }
    let summary = output
        .lines()
        .find_map(|line| xml_between(line.trim(), "<summary>", "</summary>"))
        .map(|raw| unescape_xml(&raw))
        .unwrap_or_else(|| "no children".to_string());

    let children = output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<subagent"))
        .map(|line| AgentSwarmChildRow {
            outcome: xml_attr(line, "outcome").unwrap_or_else(|| "unknown".to_string()),
            item: xml_attr(line, "item").map(|item| unescape_xml(&item)),
            agent_id: xml_attr(line, "agent_id"),
        })
        .collect();

    Some(AgentSwarmResultSummary { summary, children })
}

fn short_agent_id(agent_id: &str) -> String {
    agent_id.chars().take(8).collect()
}

fn xml_between(line: &str, open: &str, close: &str) -> Option<String> {
    let start = line.find(open)? + open.len();
    let end = line[start..].find(close)? + start;
    Some(line[start..end].to_string())
}

fn xml_attr(line: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod swarm_preview_tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "<agent_swarm_result>\n",
        "<summary>completed: 1, failed: 1</summary>\n",
        "<resume_hint>Call agent_swarm with resume_agent_ids ...</resume_hint>\n",
        "<subagent agent_id=\"5dc6a1a5-746f\" item=\"README.md\" outcome=\"completed\">ok</subagent>\n",
        "<subagent agent_id=\"52dfe84b-1281\" item=\"b &amp; c.rs\" state=\"started\" outcome=\"failed\">boom &lt;x&gt;</subagent>\n",
        "</agent_swarm_result>",
    );

    #[test]
    fn parses_summary_and_children() {
        let parsed = agent_swarm_result_summary(SAMPLE).expect("swarm summary");
        assert_eq!(parsed.summary, "completed: 1, failed: 1");
        assert_eq!(parsed.children.len(), 2);
        assert_eq!(parsed.children[0].outcome, "completed");
        assert_eq!(parsed.children[0].item.as_deref(), Some("README.md"));
        assert_eq!(
            parsed.children[0].agent_id.as_deref(),
            Some("5dc6a1a5-746f")
        );
        // XML entities in item are unescaped.
        assert_eq!(parsed.children[1].item.as_deref(), Some("b & c.rs"));
        assert_eq!(parsed.children[1].outcome, "failed");
    }

    #[test]
    fn child_label_is_compact_with_glyph_and_short_id() {
        let parsed = agent_swarm_result_summary(SAMPLE).unwrap();
        assert_eq!(parsed.children[0].glyph(), "✓");
        let label = parsed.children[0].label();
        assert!(label.starts_with("✓ README.md"));
        assert!(label.contains("(5dc6a1a5)"));
        assert_eq!(parsed.children[1].glyph(), "✗");
    }

    #[test]
    fn non_swarm_output_returns_none() {
        assert!(agent_swarm_result_summary("just some text").is_none());
    }
}
