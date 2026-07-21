//! Session filesystem operations for Claude's on-disk JSONL transcripts.

use crate::error::{ClaudeSDKError, Result};
use crate::session_store::project_key_for_directory;
use crate::sessions::{ListSessionsOptions, SessionInfo, SessionMessage};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// List all sessions from Claude's project transcript directories.
pub async fn list_sessions(opts: &ListSessionsOptions) -> Result<Vec<SessionInfo>> {
    let mut sessions = Vec::new();
    for project_dir in project_dirs(opts.directory.as_deref()) {
        let Ok(files) = std::fs::read_dir(project_dir) else {
            continue;
        };
        for path in files.flatten().map(|file| file.path()) {
            if session_id_from_jsonl_path(&path).is_some() {
                if let Some(info) = session_info_from_file(&path).await? {
                    sessions.push(info);
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if let Some(offset) = opts.offset {
        sessions = sessions.into_iter().skip(offset).collect();
    }
    if let Some(limit) = opts.limit {
        sessions.truncate(limit);
    }
    Ok(sessions)
}

/// Get information about a specific session.
pub async fn get_session_info(session_id: &str, directory: Option<&str>) -> Result<SessionInfo> {
    validate_session_id(session_id)?;
    let path = resolve_session_file(session_id, directory)
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))?;
    session_info_from_file(&path)
        .await?
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))
}

/// Get all visible user/assistant messages for a specific session.
pub async fn get_session_messages(
    session_id: &str,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<SessionMessage>> {
    validate_session_id(session_id)?;
    let Some(path) = resolve_session_file(session_id, directory) else {
        return Ok(Vec::new());
    };
    let entries = read_jsonl_entries(&path).await?;
    Ok(apply_limit_offset(
        entries
            .into_iter()
            .filter_map(session_message_from_entry)
            .collect(),
        limit,
        offset,
    ))
}

/// List subagent IDs for a specific session.
pub async fn list_subagents(session_id: &str, directory: Option<&str>) -> Result<Vec<String>> {
    validate_session_id(session_id)?;
    let Some(path) = resolve_session_file(session_id, directory) else {
        return Ok(Vec::new());
    };
    let subagents_dir = path.with_extension("").join("subagents");
    Ok(collect_agent_files(&subagents_dir)
        .into_iter()
        .map(|(agent_id, _)| agent_id)
        .collect())
}

/// Get visible user/assistant messages for a specific subagent transcript.
pub async fn get_subagent_messages(
    session_id: &str,
    agent_id: &str,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<SessionMessage>> {
    validate_session_id(session_id)?;
    if agent_id.is_empty() {
        return Ok(Vec::new());
    }
    let Some(path) = resolve_session_file(session_id, directory) else {
        return Ok(Vec::new());
    };
    let subagents_dir = path.with_extension("").join("subagents");
    let Some((_, agent_file)) = collect_agent_files(&subagents_dir)
        .into_iter()
        .find(|(found_id, _)| found_id == agent_id)
    else {
        return Ok(Vec::new());
    };
    let entries = read_jsonl_entries(&agent_file).await?;
    Ok(apply_limit_offset(
        subagent_chain(entries)
            .into_iter()
            .filter_map(session_message_from_entry)
            .collect(),
        limit,
        offset,
    ))
}

/// Rename a session by appending the same custom-title metadata line the CLI uses.
pub async fn rename_session(session_id: &str, title: &str, directory: Option<&str>) -> Result<()> {
    validate_session_id(session_id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err(ClaudeSDKError::Session(
            "title must be non-empty".to_string(),
        ));
    }
    let path = resolve_session_file(session_id, directory)
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))?;
    let entry = serde_json::json!({
        "type": "summary",
        "customTitle": title,
        "timestamp": Utc::now().to_rfc3339(),
    });
    append_jsonl_entry(&path, &entry).await
}

/// Tag a session by appending a tag metadata line. `None` clears the tag.
pub async fn tag_session(
    session_id: &str,
    tag: Option<&str>,
    directory: Option<&str>,
) -> Result<()> {
    validate_session_id(session_id)?;
    let tag = tag.map(sanitize_tag).transpose()?.unwrap_or_default();
    let path = resolve_session_file(session_id, directory)
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))?;
    let entry = serde_json::json!({
        "type": "tag",
        "tag": tag,
        "sessionId": session_id,
    });
    append_jsonl_entry(&path, &entry).await
}

/// Delete a session transcript and its sidecar subagent directory if present.
pub async fn delete_session(session_id: &str, directory: Option<&str>) -> Result<()> {
    validate_session_id(session_id)?;
    let Some(path) = resolve_session_file(session_id, directory) else {
        return Ok(());
    };
    tokio::fs::remove_file(&path).await?;
    let sidecar_dir = path.with_extension("");
    if tokio::fs::metadata(&sidecar_dir)
        .await
        .is_ok_and(|meta| meta.is_dir())
    {
        tokio::fs::remove_dir_all(sidecar_dir).await?;
    }
    Ok(())
}

fn projects_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .unwrap_or_else(|| PathBuf::from(".claude"))
        .join("projects")
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

fn apply_limit_offset(
    messages: Vec<SessionMessage>,
    limit: Option<usize>,
    offset: usize,
) -> Vec<SessionMessage> {
    let messages = if offset > 0 {
        messages.into_iter().skip(offset).collect()
    } else {
        messages
    };
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        messages.into_iter().take(limit).collect()
    } else {
        messages
    }
}

fn collect_agent_files(base_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut output = Vec::new();
    collect_agent_files_inner(base_dir, &mut output);
    output
}

fn collect_agent_files_inner(base_dir: &Path, output: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_agent_files_inner(&path, output);
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(agent_id) = file_name
            .strip_prefix("agent-")
            .and_then(|name| name.strip_suffix(".jsonl"))
        else {
            continue;
        };
        output.push((agent_id.to_string(), path));
    }
}

async fn session_info_from_file(path: &Path) -> Result<Option<SessionInfo>> {
    let Some(session_id) = session_id_from_jsonl_path(path) else {
        return Ok(None);
    };
    let entries = read_jsonl_entries(path).await?;
    if entries.is_empty() {
        return Ok(None);
    }
    let metadata = tokio::fs::metadata(path).await?;
    let updated_at = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now)
        .to_rfc3339();
    let created_at = metadata
        .created()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now)
        .to_rfc3339();
    let message_count = entries
        .iter()
        .filter(|entry| is_visible_message(entry))
        .count();
    let title = extract_title(&entries).unwrap_or_else(|| session_id.clone());

    Ok(Some(SessionInfo {
        id: session_id,
        title,
        created_at,
        updated_at,
        message_count,
    }))
}

async fn read_jsonl_entries(
    path: &Path,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>> {
    let content = tokio::fs::read_to_string(path).await?;
    let entries = content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(line).ok()
        })
        .collect();
    Ok(entries)
}

async fn append_jsonl_entry(path: &Path, entry: &serde_json::Value) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(serde_json::to_string(entry)?.as_bytes())
        .await?;
    file.write_all(b"\n").await?;
    Ok(())
}

fn session_message_from_entry(
    entry: serde_json::Map<String, serde_json::Value>,
) -> Option<SessionMessage> {
    if !is_visible_message(&entry) {
        return None;
    }
    let role = entry.get("type")?.as_str()?.to_string();
    let id = entry
        .get("uuid")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let content = entry
        .get("message")
        .and_then(|message| message.get("content"))
        .map(content_to_string)
        .unwrap_or_default();
    let timestamp = entry
        .get("timestamp")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    Some(SessionMessage {
        id,
        role,
        content,
        timestamp,
    })
}

fn subagent_chain(
    entries: Vec<serde_json::Map<String, serde_json::Value>>,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let Some(leaf_uuid) = entries
        .iter()
        .rev()
        .find(|entry| matches!(entry_type(entry), Some("user" | "assistant")))
        .and_then(|entry| entry.get("uuid"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
    else {
        return Vec::new();
    };
    let by_uuid = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("uuid")
                .and_then(|value| value.as_str())
                .map(|uuid| (uuid.to_string(), entry.clone()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current = Some(leaf_uuid);
    while let Some(uuid) = current {
        if !seen.insert(uuid.clone()) {
            break;
        }
        let Some(entry) = by_uuid.get(&uuid).cloned() else {
            break;
        };
        current = entry
            .get("parentUuid")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        chain.push(entry);
    }
    chain.reverse();
    chain
}

fn extract_title(entries: &[serde_json::Map<String, serde_json::Value>]) -> Option<String> {
    entries
        .iter()
        .rev()
        .find_map(|entry| string_field(entry, "customTitle"))
        .or_else(|| {
            entries
                .iter()
                .rev()
                .find_map(|entry| string_field(entry, "summary"))
        })
        .or_else(|| {
            entries
                .iter()
                .find(|entry| is_visible_message(entry) && entry_type(entry) == Some("user"))
                .and_then(|entry| entry.get("message"))
                .and_then(|message| message.get("content"))
                .map(content_to_string)
                .map(truncate_title)
        })
}

fn is_visible_message(entry: &serde_json::Map<String, serde_json::Value>) -> bool {
    matches!(entry_type(entry), Some("user" | "assistant"))
        && !bool_field(entry, "isMeta")
        && !bool_field(entry, "isSidechain")
        && !entry.contains_key("teamName")
}

fn entry_type(entry: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
    entry.get("type").and_then(|value| value.as_str())
}

fn string_field(entry: &serde_json::Map<String, serde_json::Value>, field: &str) -> Option<String> {
    entry
        .get(field)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

fn bool_field(entry: &serde_json::Map<String, serde_json::Value>, field: &str) -> bool {
    entry
        .get(field)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn sanitize_tag(tag: &str) -> Result<String> {
    let sanitized = tag
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    if sanitized.is_empty() {
        Err(ClaudeSDKError::Session(
            "tag must be non-empty (use None to clear)".to_string(),
        ))
    } else {
        Ok(sanitized)
    }
}

fn content_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|value| value.as_str()) == Some("text") {
                    block
                        .get("text")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

fn truncate_title(value: String) -> String {
    let normalized = value.replace('\n', " ").trim().to_string();
    if normalized.chars().count() <= 200 {
        normalized
    } else {
        format!("{}...", normalized.chars().take(200).collect::<String>())
    }
}

fn session_id_from_jsonl_path(path: &Path) -> Option<String> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return None;
    }
    let session_id = path.file_stem()?.to_str()?;
    uuid::Uuid::parse_str(session_id).ok()?;
    Some(session_id.to_string())
}

fn validate_session_id(session_id: &str) -> Result<()> {
    uuid::Uuid::parse_str(session_id)
        .map(|_| ())
        .map_err(|_| ClaudeSDKError::Session(format!("Invalid session_id: {session_id}")))
}
