use crate::error::{ClaudeSDKError, Result};
use crate::session_store::{
    project_key_for_directory, SessionKey, SessionStoreEntry, SessionStoreHandle,
};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSessionResult {
    pub session_id: String,
}

pub async fn fork_session_via_store(
    session_store: &SessionStoreHandle,
    session_id: &str,
    directory: Option<&str>,
    up_to_message_id: Option<&str>,
    title: Option<&str>,
) -> Result<ForkSessionResult> {
    validate_uuid("session_id", session_id)?;
    if let Some(up_to_message_id) = up_to_message_id {
        validate_uuid("up_to_message_id", up_to_message_id)?;
    }

    let project_key = project_key_for_directory(directory.map(Path::new));
    let source_key = SessionKey {
        project_key: project_key.clone(),
        session_id: session_id.to_string(),
        subpath: None,
    };
    let Some(entries) = session_store.load(source_key).await? else {
        return Err(ClaudeSDKError::Session(format!(
            "Session {session_id} not found"
        )));
    };
    let fork_entries = fork_entries(entries, session_id, up_to_message_id, title)?;
    let forked_session_id = fork_entries
        .first()
        .and_then(|entry| entry.get("sessionId").or_else(|| entry.get("session_id")))
        .and_then(|value| value.as_str())
        .ok_or_else(|| ClaudeSDKError::Session("fork produced no session id".to_string()))?
        .to_string();

    session_store
        .append(
            SessionKey {
                project_key,
                session_id: forked_session_id.clone(),
                subpath: None,
            },
            fork_entries,
        )
        .await?;

    Ok(ForkSessionResult {
        session_id: forked_session_id,
    })
}

fn fork_entries(
    entries: Vec<SessionStoreEntry>,
    source_session_id: &str,
    up_to_message_id: Option<&str>,
    title: Option<&str>,
) -> Result<Vec<SessionStoreEntry>> {
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
    entries: Vec<SessionStoreEntry>,
    up_to_message_id: Option<&str>,
) -> Result<Vec<SessionStoreEntry>> {
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
    mut entry: SessionStoreEntry,
    source_session_id: &str,
    forked_session_id: &str,
    uuid_map: &HashMap<String, String>,
) -> SessionStoreEntry {
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
    entry: &mut SessionStoreEntry,
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

fn custom_title_entry(session_id: &str, title: &str) -> SessionStoreEntry {
    let mut entry = serde_json::Map::new();
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

fn validate_uuid(name: &str, value: &str) -> Result<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ClaudeSDKError::Session(format!("Invalid {name}: {value}")))
}
