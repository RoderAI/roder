use crate::session_store::{SessionKey, SessionStoreEntry, SessionSummaryEntry};
use crate::sessions::SDKSessionInfo;

const LAST_WINS_FIELDS: &[(&str, &str)] = &[
    ("customTitle", "custom_title"),
    ("aiTitle", "ai_title"),
    ("lastPrompt", "last_prompt"),
    ("summary", "summary_hint"),
    ("gitBranch", "git_branch"),
];

pub fn fold_session_summary(
    prev: Option<&SessionSummaryEntry>,
    key: &SessionKey,
    entries: &[SessionStoreEntry],
) -> SessionSummaryEntry {
    let mut summary = prev.cloned().unwrap_or_else(|| SessionSummaryEntry {
        session_id: key.session_id.clone(),
        mtime: 0,
        data: serde_json::Map::new(),
    });

    for entry in entries {
        fold_entry(&mut summary.data, entry);
    }

    summary
}

pub fn summary_entry_to_sdk_info(
    entry: &SessionSummaryEntry,
    project_path: Option<&str>,
) -> Option<SDKSessionInfo> {
    let data = &entry.data;
    if data
        .get("is_sidechain")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let first_prompt = if data
        .get("first_prompt_locked")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        string_data(data, "first_prompt")
    } else {
        string_data(data, "command_fallback")
    };
    let custom_title = string_data(data, "custom_title").or_else(|| string_data(data, "ai_title"));
    let summary = custom_title
        .clone()
        .or_else(|| string_data(data, "last_prompt"))
        .or_else(|| string_data(data, "summary_hint"))
        .or_else(|| first_prompt.clone())?;

    Some(SDKSessionInfo {
        session_id: entry.session_id.clone(),
        summary,
        last_modified: entry.mtime,
        file_size: None,
        custom_title,
        first_prompt,
        git_branch: string_data(data, "git_branch"),
        cwd: string_data(data, "cwd").or_else(|| project_path.map(String::from)),
        tag: string_data(data, "tag"),
        created_at: data.get("created_at").and_then(|value| value.as_i64()),
    })
}

fn string_data(data: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    data.get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn fold_entry(data: &mut serde_json::Map<String, serde_json::Value>, entry: &SessionStoreEntry) {
    if !data.contains_key("is_sidechain") {
        data.insert(
            "is_sidechain".to_string(),
            serde_json::Value::Bool(
                entry.get("isSidechain").and_then(|v| v.as_bool()) == Some(true),
            ),
        );
    }

    if !data.contains_key("created_at") {
        if let Some(timestamp) = entry.get("timestamp").and_then(|v| v.as_str()) {
            if let Some(ms) = iso_to_epoch_ms(timestamp) {
                data.insert("created_at".to_string(), serde_json::json!(ms));
            }
        }
    }

    if !data.contains_key("cwd") {
        if let Some(cwd) = entry
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|cwd| !cwd.is_empty())
        {
            data.insert(
                "cwd".to_string(),
                serde_json::Value::String(cwd.to_string()),
            );
        }
    }

    fold_first_prompt(data, entry);

    for (src, dst) in LAST_WINS_FIELDS {
        if let Some(value) = entry.get(*src).and_then(|v| v.as_str()) {
            data.insert(
                (*dst).to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    if entry.get("type").and_then(|v| v.as_str()) == Some("tag") {
        match entry
            .get("tag")
            .and_then(|v| v.as_str())
            .filter(|tag| !tag.is_empty())
        {
            Some(tag) => {
                data.insert(
                    "tag".to_string(),
                    serde_json::Value::String(tag.to_string()),
                );
            }
            None => {
                data.remove("tag");
            }
        }
    }
}

fn iso_to_epoch_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn fold_first_prompt(
    data: &mut serde_json::Map<String, serde_json::Value>,
    entry: &SessionStoreEntry,
) {
    if data
        .get("first_prompt_locked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return;
    }
    if entry.get("type").and_then(|v| v.as_str()) != Some("user") {
        return;
    }
    if entry.get("isMeta").and_then(|v| v.as_bool()) == Some(true)
        || entry.get("isCompactSummary").and_then(|v| v.as_bool()) == Some(true)
        || carries_tool_result(entry)
    {
        return;
    }

    for raw in entry_text_blocks(entry) {
        let mut prompt = raw.replace('\n', " ").trim().to_string();
        if prompt.is_empty() {
            continue;
        }
        if let Some(command) = command_name(&prompt) {
            data.entry("command_fallback".to_string())
                .or_insert_with(|| serde_json::Value::String(command));
            continue;
        }
        if should_skip_first_prompt(&prompt) {
            continue;
        }
        if prompt.len() > 200 {
            prompt.truncate(200);
            prompt = prompt.trim_end().to_string();
            prompt.push('\u{2026}');
        }
        data.insert(
            "first_prompt".to_string(),
            serde_json::Value::String(prompt),
        );
        data.insert(
            "first_prompt_locked".to_string(),
            serde_json::Value::Bool(true),
        );
        return;
    }
}

fn entry_text_blocks(entry: &SessionStoreEntry) -> Vec<String> {
    let Some(message) = entry.get("message").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    match message.get("content") {
        Some(serde_json::Value::String(text)) => vec![text.clone()],
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                let block = block.as_object()?;
                (block.get("type").and_then(|v| v.as_str()) == Some("text"))
                    .then(|| block.get("text").and_then(|v| v.as_str()).map(String::from))
                    .flatten()
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn carries_tool_result(entry: &SessionStoreEntry) -> bool {
    entry
        .get("message")
        .and_then(|v| v.as_object())
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_array())
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                block
                    .as_object()
                    .and_then(|block| block.get("type"))
                    .and_then(|v| v.as_str())
                    == Some("tool_result")
            })
        })
}

fn command_name(prompt: &str) -> Option<String> {
    let start = prompt.find("<command-name>")? + "<command-name>".len();
    let end = prompt[start..].find("</command-name>")? + start;
    Some(prompt[start..end].to_string())
}

fn should_skip_first_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim_start();
    trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<session-start-hook>")
        || trimmed.starts_with("<tick>")
        || trimmed.starts_with("<goal>")
        || trimmed.starts_with("[Request interrupted by user")
        || (trimmed.starts_with("<ide_opened_file>") && trimmed.ends_with("</ide_opened_file>"))
        || (trimmed.starts_with("<ide_selection>") && trimmed.ends_with("</ide_selection>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SessionKey {
        SessionKey {
            project_key: "proj".to_string(),
            session_id: "session".to_string(),
            subpath: None,
        }
    }

    #[test]
    fn folds_first_prompt_and_last_wins_fields() {
        let entries = vec![
            serde_json::json!({
                "type": "user",
                "timestamp": "2026-05-08T10:00:00Z",
                "cwd": "/repo",
                "message": {"content": "<command-name>init</command-name>"}
            })
            .as_object()
            .unwrap()
            .clone(),
            serde_json::json!({
                "type": "user",
                "lastPrompt": "latest prompt",
                "gitBranch": "main",
                "message": {"content": [{"type": "text", "text": "Real prompt"}]}
            })
            .as_object()
            .unwrap()
            .clone(),
        ];

        let summary = fold_session_summary(None, &key(), &entries);

        assert_eq!(summary.session_id, "session");
        assert_eq!(summary.data["created_at"], 1_778_234_400_000i64);
        assert_eq!(summary.data["cwd"], "/repo");
        assert_eq!(summary.data["command_fallback"], "init");
        assert_eq!(summary.data["first_prompt"], "Real prompt");
        assert_eq!(summary.data["last_prompt"], "latest prompt");
        assert_eq!(summary.data["git_branch"], "main");
    }

    #[test]
    fn tag_entries_set_and_clear_tag() {
        let set_tag = serde_json::json!({"type": "tag", "tag": "important"})
            .as_object()
            .unwrap()
            .clone();
        let clear_tag = serde_json::json!({"type": "tag", "tag": ""})
            .as_object()
            .unwrap()
            .clone();

        let with_tag = fold_session_summary(None, &key(), &[set_tag]);
        assert_eq!(with_tag.data["tag"], "important");

        let without_tag = fold_session_summary(Some(&with_tag), &key(), &[clear_tag]);
        assert!(without_tag.data.get("tag").is_none());
    }
}
