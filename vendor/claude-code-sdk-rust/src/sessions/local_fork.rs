use crate::error::{ClaudeSDKError, Result};
use crate::session_store::project_key_for_directory;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalForkSessionResult {
    pub session_id: String,
}

pub async fn fork_session(
    session_id: &str,
    directory: Option<&str>,
    up_to_message_id: Option<&str>,
    title: Option<&str>,
) -> Result<LocalForkSessionResult> {
    validate_uuid("session_id", session_id)?;
    if let Some(up_to_message_id) = up_to_message_id {
        validate_uuid("up_to_message_id", up_to_message_id)?;
    }

    let source_path = resolve_session_file(session_id, directory)
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))?;
    let entries = read_jsonl_entries(&source_path).await?;
    let forked_entries = fork_entries(entries, session_id, up_to_message_id, title)?;
    let forked_session_id = forked_entries
        .first()
        .and_then(|entry| entry.get("sessionId").or_else(|| entry.get("session_id")))
        .and_then(|value| value.as_str())
        .ok_or_else(|| ClaudeSDKError::Session("fork produced no session id".to_string()))?
        .to_string();
    let fork_path = source_path.with_file_name(format!("{forked_session_id}.jsonl"));
    write_jsonl_entries(&fork_path, &forked_entries).await?;
    Ok(LocalForkSessionResult {
        session_id: forked_session_id,
    })
}

fn fork_entries(
    entries: Vec<Map<String, Value>>,
    source_session_id: &str,
    up_to_message_id: Option<&str>,
    title: Option<&str>,
) -> Result<Vec<Map<String, Value>>> {
    let selected = select_entries(entries, up_to_message_id)?;
    if selected.is_empty() {
        return Err(ClaudeSDKError::Session(
            "session has no messages to fork".to_string(),
        ));
    }

    let forked_session_id = uuid::Uuid::new_v4().to_string();
    let uuid_map = selected
        .iter()
        .filter_map(|entry| entry.get("uuid").and_then(|value| value.as_str()))
        .map(|old| (old.to_string(), uuid::Uuid::new_v4().to_string()))
        .collect::<HashMap<_, _>>();

    let mut forked = selected
        .into_iter()
        .map(|entry| remap_entry(entry, source_session_id, &forked_session_id, &uuid_map))
        .collect::<Vec<_>>();
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        forked.push(custom_title_entry(&forked_session_id, title));
    }
    Ok(forked)
}

fn select_entries(
    entries: Vec<Map<String, Value>>,
    up_to_message_id: Option<&str>,
) -> Result<Vec<Map<String, Value>>> {
    let Some(up_to_message_id) = up_to_message_id else {
        return Ok(entries);
    };
    let mut selected = Vec::new();
    let mut found = false;
    for entry in entries {
        let matches_target =
            entry.get("uuid").and_then(|value| value.as_str()) == Some(up_to_message_id);
        selected.push(entry);
        if matches_target {
            found = true;
            break;
        }
    }
    if found {
        Ok(selected)
    } else {
        Err(ClaudeSDKError::Session(format!(
            "Message {up_to_message_id} not found"
        )))
    }
}

fn remap_entry(
    mut entry: Map<String, Value>,
    source_session_id: &str,
    forked_session_id: &str,
    uuid_map: &HashMap<String, String>,
) -> Map<String, Value> {
    replace_uuid_field(&mut entry, "uuid", uuid_map);
    replace_uuid_field(&mut entry, "parentUuid", uuid_map);
    replace_uuid_field(&mut entry, "parent_uuid", uuid_map);
    replace_uuid_field(&mut entry, "parent_tool_use_id", uuid_map);
    entry.insert(
        "sessionId".to_string(),
        serde_json::json!(forked_session_id),
    );
    entry.insert(
        "session_id".to_string(),
        serde_json::json!(forked_session_id),
    );
    entry.insert(
        "forkedFrom".to_string(),
        serde_json::json!(source_session_id),
    );
    entry
}

fn replace_uuid_field(
    entry: &mut Map<String, Value>,
    field: &str,
    uuid_map: &HashMap<String, String>,
) {
    let Some(old) = entry.get(field).and_then(|value| value.as_str()) else {
        return;
    };
    if let Some(new) = uuid_map.get(old) {
        entry.insert(field.to_string(), serde_json::json!(new));
    }
}

fn custom_title_entry(session_id: &str, title: &str) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("type".to_string(), serde_json::json!("custom-title"));
    entry.insert("customTitle".to_string(), serde_json::json!(title));
    entry.insert("sessionId".to_string(), serde_json::json!(session_id));
    entry.insert("session_id".to_string(), serde_json::json!(session_id));
    entry.insert(
        "uuid".to_string(),
        serde_json::json!(uuid::Uuid::new_v4().to_string()),
    );
    entry.insert(
        "timestamp".to_string(),
        serde_json::json!(chrono::Utc::now().to_rfc3339()),
    );
    entry
}

fn resolve_session_file(session_id: &str, directory: Option<&str>) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    for project in project_dirs(directory) {
        let candidate = project.join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn project_dirs(directory: Option<&str>) -> Vec<PathBuf> {
    if let Some(directory) = directory {
        let dir = projects_dir().join(project_key_for_directory(Some(Path::new(directory))));
        return if dir.is_dir() { vec![dir] } else { Vec::new() };
    }
    let Ok(projects) = std::fs::read_dir(projects_dir()) else {
        return Vec::new();
    };
    projects
        .flatten()
        .filter(|project| project.file_type().ok().is_some_and(|ty| ty.is_dir()))
        .map(|project| project.path())
        .collect()
}

fn projects_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .unwrap_or_else(|| PathBuf::from(".claude"))
        .join("projects")
}

async fn read_jsonl_entries(path: &Path) -> Result<Vec<Map<String, Value>>> {
    let content = tokio::fs::read_to_string(path).await?;
    Ok(content
        .lines()
        .filter_map(|line| serde_json::from_str::<Map<String, Value>>(line).ok())
        .collect())
}

async fn write_jsonl_entries(path: &Path, entries: &[Map<String, Value>]) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await?;
    for entry in entries {
        file.write_all(serde_json::to_string(entry)?.as_bytes())
            .await?;
        file.write_all(b"\n").await?;
    }
    Ok(())
}

fn validate_uuid(name: &str, value: &str) -> Result<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ClaudeSDKError::Session(format!("Invalid {name}: {value}")))
}
