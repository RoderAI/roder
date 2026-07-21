use crate::error::{ClaudeSDKError, Result};
use crate::session_store::{
    project_key_for_directory, SessionKey, SessionStoreEntry, SessionStoreHandle,
};
use crate::session_summary::{fold_session_summary, summary_entry_to_sdk_info};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SDKSessionInfo {
    pub session_id: String,
    pub summary: String,
    pub last_modified: i64,
    pub file_size: Option<i64>,
    pub custom_title: Option<String>,
    pub first_prompt: Option<String>,
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub tag: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SDKSessionMessage {
    pub r#type: String,
    pub uuid: String,
    pub session_id: String,
    pub message: serde_json::Value,
    pub parent_tool_use_id: Option<String>,
}

pub async fn rename_session_via_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    title: &str,
    directory: Option<&str>,
) -> Result<()> {
    validate_session_id(session_id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err(ClaudeSDKError::Session(
            "title must be non-empty".to_string(),
        ));
    }
    let project_key = project_key_for_directory(directory.map(Path::new));
    session_store
        .append(
            SessionKey {
                project_key,
                session_id: session_id.to_string(),
                subpath: None,
            },
            vec![metadata_entry(
                "custom-title",
                session_id,
                [("customTitle", serde_json::Value::String(title.to_string()))],
            )],
        )
        .await
}

pub async fn tag_session_via_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    tag: Option<&str>,
    directory: Option<&str>,
) -> Result<()> {
    validate_session_id(session_id)?;
    let tag = tag.map(sanitize_tag).transpose()?;
    let project_key = project_key_for_directory(directory.map(Path::new));
    session_store
        .append(
            SessionKey {
                project_key,
                session_id: session_id.to_string(),
                subpath: None,
            },
            vec![metadata_entry(
                "tag",
                session_id,
                [("tag", serde_json::Value::String(tag.unwrap_or_default()))],
            )],
        )
        .await
}

pub async fn delete_session_via_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    directory: Option<&str>,
) -> Result<()> {
    validate_session_id(session_id)?;
    let project_key = project_key_for_directory(directory.map(Path::new));
    session_store
        .delete(SessionKey {
            project_key,
            session_id: session_id.to_string(),
            subpath: None,
        })
        .await
}

pub async fn list_sessions_from_store(
    session_store: &SessionStoreHandle,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<SDKSessionInfo>> {
    let project_path = canonical_project_path(directory);
    let project_key = project_key_for_directory(Some(Path::new(&project_path)));
    let listing = session_store.list_sessions(&project_key).await?;
    let summaries = session_store
        .list_session_summaries(&project_key)
        .await
        .unwrap_or_default();

    let mut infos = Vec::new();
    let mut covered = std::collections::HashSet::new();
    let known_mtimes = listing
        .iter()
        .map(|entry| (entry.session_id.clone(), entry.mtime))
        .collect::<std::collections::HashMap<_, _>>();

    for summary in summaries {
        if let Some(known_mtime) = known_mtimes.get(&summary.session_id) {
            if summary.mtime < *known_mtime {
                continue;
            }
        }
        covered.insert(summary.session_id.clone());
        if let Some(info) = summary_entry_to_sdk_info(&summary, Some(&project_path)) {
            infos.push(info);
        }
    }

    for entry in listing {
        if covered.contains(&entry.session_id) {
            continue;
        }
        match derive_session_info_from_store(
            session_store,
            &project_key,
            &project_path,
            &entry.session_id,
        )
        .await
        {
            Ok(Some(mut info)) => {
                info.last_modified = entry.mtime;
                infos.push(info);
            }
            Ok(None) => {}
            Err(_) => infos.push(degraded_session_info(&entry.session_id, entry.mtime)),
        }
    }

    Ok(sort_limit_offset(infos, limit, offset))
}

fn degraded_session_info(session_id: &str, last_modified: i64) -> SDKSessionInfo {
    SDKSessionInfo {
        session_id: session_id.to_string(),
        summary: String::new(),
        last_modified,
        file_size: None,
        custom_title: None,
        first_prompt: None,
        git_branch: None,
        cwd: None,
        tag: None,
        created_at: None,
    }
}

pub async fn get_session_info_from_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    directory: Option<&str>,
) -> Result<Option<SDKSessionInfo>> {
    if !is_uuid(session_id) {
        return Ok(None);
    }
    let project_path = canonical_project_path(directory);
    let project_key = project_key_for_directory(Some(Path::new(&project_path)));
    derive_session_info_from_store(session_store, &project_key, &project_path, session_id).await
}

pub async fn get_session_messages_from_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<SDKSessionMessage>> {
    if !is_uuid(session_id) {
        return Ok(Vec::new());
    }
    let project_key = project_key_for_directory(directory.map(Path::new));
    let key = SessionKey {
        project_key,
        session_id: session_id.to_string(),
        subpath: None,
    };
    let Some(entries) = session_store.load(key).await? else {
        return Ok(Vec::new());
    };
    Ok(entries_to_session_messages(
        session_id, entries, limit, offset,
    ))
}

pub async fn list_subagents_from_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    directory: Option<&str>,
) -> Result<Vec<String>> {
    if !is_uuid(session_id) {
        return Ok(Vec::new());
    }
    let project_key = project_key_for_directory(directory.map(Path::new));
    let subkeys = session_store
        .list_subkeys(crate::session_store::SessionListSubkeysKey {
            project_key,
            session_id: session_id.to_string(),
        })
        .await?;
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::new();
    for subkey in subkeys {
        if let Some(agent_id) = subagent_id_from_subkey(&subkey) {
            if seen.insert(agent_id.to_string()) {
                ids.push(agent_id.to_string());
            }
        }
    }
    Ok(ids)
}

pub async fn get_subagent_messages_from_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    agent_id: &str,
    directory: Option<&str>,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<SDKSessionMessage>> {
    if !is_uuid(session_id) || agent_id.is_empty() {
        return Ok(Vec::new());
    }
    let project_key = project_key_for_directory(directory.map(Path::new));
    let subpath = find_subagent_subpath(session_store, &project_key, session_id, agent_id).await?;
    let Some(entries) = session_store
        .load(SessionKey {
            project_key,
            session_id: session_id.to_string(),
            subpath: Some(subpath),
        })
        .await?
    else {
        return Ok(Vec::new());
    };
    let entries = entries
        .into_iter()
        .filter(|entry| {
            entry.get("type").and_then(|value| value.as_str()) != Some("agent_metadata")
        })
        .collect();
    Ok(entries_to_session_messages(
        session_id, entries, limit, offset,
    ))
}

async fn find_subagent_subpath(
    session_store: &SessionStoreHandle,
    project_key: &str,
    session_id: &str,
    agent_id: &str,
) -> Result<String> {
    let target = format!("agent-{agent_id}");
    let subkeys = session_store
        .list_subkeys(crate::session_store::SessionListSubkeysKey {
            project_key: project_key.to_string(),
            session_id: session_id.to_string(),
        })
        .await
        .unwrap_or_default();
    Ok(subkeys
        .into_iter()
        .find(|subkey| {
            subkey.starts_with("subagents/") && subkey.rsplit('/').next() == Some(target.as_str())
        })
        .unwrap_or_else(|| format!("subagents/{target}")))
}

async fn derive_session_info_from_store(
    session_store: &SessionStoreHandle,
    project_key: &str,
    project_path: &str,
    session_id: &str,
) -> Result<Option<SDKSessionInfo>> {
    let key = SessionKey {
        project_key: project_key.to_string(),
        session_id: session_id.to_string(),
        subpath: None,
    };
    let Some(entries) = session_store.load(key.clone()).await? else {
        return Ok(None);
    };
    if entries.is_empty() {
        return Ok(None);
    }
    let summary = fold_session_summary(None, &key, &entries);
    Ok(summary_entry_to_sdk_info(&summary, Some(project_path)))
}

fn entries_to_session_messages(
    session_id: &str,
    entries: Vec<SessionStoreEntry>,
    limit: Option<usize>,
    offset: usize,
) -> Vec<SDKSessionMessage> {
    entries
        .into_iter()
        .filter_map(|entry| session_message_from_entry(session_id, entry))
        .skip(offset)
        .take(limit.filter(|limit| *limit > 0).unwrap_or(usize::MAX))
        .collect()
}

fn session_message_from_entry(
    fallback_session_id: &str,
    entry: SessionStoreEntry,
) -> Option<SDKSessionMessage> {
    let message_type = entry.get("type")?.as_str()?;
    if message_type != "user" && message_type != "assistant" {
        return None;
    }
    let uuid = entry.get("uuid")?.as_str()?.to_string();
    let message = entry.get("message")?.clone();
    Some(SDKSessionMessage {
        r#type: message_type.to_string(),
        uuid,
        session_id: entry
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or(fallback_session_id)
            .to_string(),
        message,
        parent_tool_use_id: entry
            .get("parent_tool_use_id")
            .and_then(|value| value.as_str())
            .map(String::from),
    })
}

fn subagent_id_from_subkey(subkey: &str) -> Option<&str> {
    if !subkey.starts_with("subagents/") {
        return None;
    }
    subkey.rsplit('/').next()?.strip_prefix("agent-")
}

fn sort_limit_offset(
    mut infos: Vec<SDKSessionInfo>,
    limit: Option<usize>,
    offset: usize,
) -> Vec<SDKSessionInfo> {
    infos.sort_by_key(|info| std::cmp::Reverse(info.last_modified));
    let infos = if offset > 0 {
        infos.into_iter().skip(offset).collect()
    } else {
        infos
    };
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        infos.into_iter().take(limit).collect()
    } else {
        infos
    }
}

fn canonical_project_path(directory: Option<&str>) -> String {
    let path = directory.map(Path::new).unwrap_or_else(|| Path::new("."));
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| ".".into())
            .join(path)
    };
    std::fs::canonicalize(&absolute)
        .unwrap_or(absolute)
        .to_string_lossy()
        .to_string()
}

fn is_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if is_uuid(session_id) {
        Ok(())
    } else {
        Err(ClaudeSDKError::Session(format!(
            "Invalid session_id: {session_id}"
        )))
    }
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

fn metadata_entry<const N: usize>(
    entry_type: &str,
    session_id: &str,
    fields: [(&str, serde_json::Value); N],
) -> SessionStoreEntry {
    let mut entry = serde_json::Map::new();
    entry.insert("type".to_string(), serde_json::json!(entry_type));
    entry.insert("sessionId".to_string(), serde_json::json!(session_id));
    entry.insert(
        "uuid".to_string(),
        serde_json::json!(uuid::Uuid::new_v4().to_string()),
    );
    entry.insert(
        "timestamp".to_string(),
        serde_json::json!(chrono::Utc::now().to_rfc3339()),
    );
    for (key, value) in fields {
        entry.insert(key.to_string(), value);
    }
    entry
}
