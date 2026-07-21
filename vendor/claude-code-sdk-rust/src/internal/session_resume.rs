use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::error::{ClaudeSDKError, Result};
use crate::session_store::{
    project_key_for_directory, SessionKey, SessionListSubkeysKey, SessionStoreEntry,
    SessionStoreHandle,
};
use crate::types::ClaudeAgentOptions;

#[derive(Debug, Clone)]
pub struct MaterializedResume {
    pub config_dir: PathBuf,
    pub resume_session_id: String,
}

impl MaterializedResume {
    pub async fn cleanup(&self) {
        let _ = tokio::fs::remove_dir_all(&self.config_dir).await;
    }
}

pub async fn materialize_resume_session(
    options: &ClaudeAgentOptions,
) -> Result<Option<MaterializedResume>> {
    let Some(store) = options.session_store.clone() else {
        return Ok(None);
    };
    if options.resume.is_none() && !options.continue_conversation {
        return Ok(None);
    }

    let project_key = project_key_for_directory(options.cwd.as_deref().map(Path::new));
    let timeout = Duration::from_millis(options.load_timeout_ms.max(0) as u64);
    let resolved = if let Some(session_id) = options.resume.as_ref() {
        if !is_uuid(session_id) {
            return Ok(None);
        }
        load_candidate(&store, &project_key, session_id, timeout).await?
    } else {
        resolve_continue_candidate(&store, &project_key, timeout).await?
    };

    let Some((session_id, entries)) = resolved else {
        return Ok(None);
    };

    let config_dir =
        std::env::temp_dir().join(format!("claude-resume-rust-{}", uuid::Uuid::new_v4()));
    let project_dir = config_dir.join("projects").join(&project_key);
    tokio::fs::create_dir_all(&project_dir).await?;
    if let Err(error) =
        write_jsonl(&project_dir.join(format!("{session_id}.jsonl")), &entries).await
    {
        let _ = tokio::fs::remove_dir_all(&config_dir).await;
        return Err(error);
    }

    if let Err(error) =
        materialize_subkeys(&store, &project_dir, &project_key, &session_id, timeout).await
    {
        let _ = tokio::fs::remove_dir_all(&config_dir).await;
        return Err(error);
    }

    copy_auth_files(&config_dir, &options.env).await;

    Ok(Some(MaterializedResume {
        config_dir,
        resume_session_id: session_id,
    }))
}

pub fn apply_materialized_options(
    options: &ClaudeAgentOptions,
    materialized: &MaterializedResume,
) -> ClaudeAgentOptions {
    let mut options = options.clone();
    options.env.insert(
        "CLAUDE_CONFIG_DIR".to_string(),
        materialized.config_dir.to_string_lossy().to_string(),
    );
    options.resume = Some(materialized.resume_session_id.clone());
    options.continue_conversation = false;
    options
}

async fn load_candidate(
    store: &SessionStoreHandle,
    project_key: &str,
    session_id: &str,
    timeout: Duration,
) -> Result<Option<(String, Vec<SessionStoreEntry>)>> {
    let key = SessionKey {
        project_key: project_key.to_string(),
        session_id: session_id.to_string(),
        subpath: None,
    };
    let entries = with_timeout(
        store.load(key),
        timeout,
        format!("SessionStore.load() for session {session_id}"),
    )
    .await?;
    Ok(entries
        .filter(|entries| !entries.is_empty())
        .map(|entries| (session_id.to_string(), entries)))
}

async fn resolve_continue_candidate(
    store: &SessionStoreHandle,
    project_key: &str,
    timeout: Duration,
) -> Result<Option<(String, Vec<SessionStoreEntry>)>> {
    let mut sessions = with_timeout(
        store.list_sessions(project_key),
        timeout,
        "SessionStore.list_sessions()".to_string(),
    )
    .await?;
    sessions.sort_by_key(|session| std::cmp::Reverse(session.mtime));

    for session in sessions {
        if !is_uuid(&session.session_id) {
            continue;
        }
        let Some((session_id, entries)) =
            load_candidate(store, project_key, &session.session_id, timeout).await?
        else {
            continue;
        };
        if entries
            .first()
            .and_then(|entry| entry.get("isSidechain"))
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            continue;
        }
        return Ok(Some((session_id, entries)));
    }

    Ok(None)
}

async fn materialize_subkeys(
    store: &SessionStoreHandle,
    project_dir: &Path,
    project_key: &str,
    session_id: &str,
    timeout: Duration,
) -> Result<()> {
    let subkeys = store
        .list_subkeys(SessionListSubkeysKey {
            project_key: project_key.to_string(),
            session_id: session_id.to_string(),
        })
        .await
        .unwrap_or_default();
    let session_dir = project_dir.join(session_id);

    for subpath in subkeys {
        if !is_safe_subpath(&subpath) {
            continue;
        }
        let key = SessionKey {
            project_key: project_key.to_string(),
            session_id: session_id.to_string(),
            subpath: Some(subpath.clone()),
        };
        let Some(entries) = with_timeout(
            store.load(key),
            timeout,
            format!("SessionStore.load() for session {session_id} subpath {subpath}"),
        )
        .await?
        else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }

        write_subpath_entries(&session_dir, &subpath, &entries).await?;
    }
    Ok(())
}

async fn write_subpath_entries(
    session_dir: &Path,
    subpath: &str,
    entries: &[SessionStoreEntry],
) -> Result<()> {
    let mut transcript = Vec::new();
    let mut metadata = None;
    for entry in entries {
        if entry.get("type").and_then(|value| value.as_str()) == Some("agent_metadata") {
            let mut metadata_entry = entry.clone();
            metadata_entry.remove("type");
            metadata = Some(metadata_entry);
        } else {
            transcript.push(entry.clone());
        }
    }

    let base_path = session_dir.join(subpath);
    let transcript_path = base_path.with_extension("jsonl");
    if !transcript.is_empty() {
        write_jsonl(&transcript_path, &transcript).await?;
    }
    if let Some(metadata) = metadata {
        let metadata_path = base_path.with_extension("meta.json");
        if let Some(parent) = metadata_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&metadata_path, serde_json::to_vec(&metadata)?).await?;
    }
    Ok(())
}

async fn write_jsonl(path: &Path, entries: &[SessionStoreEntry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut out = Vec::new();
    for entry in entries {
        out.extend(serde_json::to_vec(entry)?);
        out.push(b'\n');
    }
    tokio::fs::write(path, out).await?;
    Ok(())
}

async fn with_timeout<F, T>(future: F, timeout: Duration, label: String) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result.map_err(|error| {
            ClaudeSDKError::Session(format!(
                "{label} failed during resume materialization: {error}"
            ))
        }),
        Err(_) => Err(ClaudeSDKError::Session(format!(
            "{label} timed out after {}ms during resume materialization",
            timeout.as_millis()
        ))),
    }
}

async fn copy_auth_files(config_dir: &Path, env: &std::collections::HashMap<String, String>) {
    let source_config_dir = env
        .get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")));

    if let Some(source_config_dir) = source_config_dir {
        copy_redacted_credentials(
            &source_config_dir.join(".credentials.json"),
            &config_dir.join(".credentials.json"),
        )
        .await;
        let claude_json = if env.contains_key("CLAUDE_CONFIG_DIR")
            || std::env::var_os("CLAUDE_CONFIG_DIR").is_some()
        {
            source_config_dir.join(".claude.json")
        } else {
            dirs::home_dir()
                .map(|home| home.join(".claude.json"))
                .unwrap_or_else(|| PathBuf::from(".claude.json"))
        };
        copy_if_present(&claude_json, &config_dir.join(".claude.json")).await;
    }
}

async fn copy_redacted_credentials(src: &Path, dst: &Path) {
    let Ok(content) = tokio::fs::read_to_string(src).await else {
        return;
    };
    let redacted = redact_refresh_token(&content).unwrap_or(content);
    let _ = tokio::fs::write(dst, redacted).await;
}

fn redact_refresh_token(content: &str) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    value
        .get_mut("claudeAiOauth")?
        .as_object_mut()?
        .remove("refreshToken");
    serde_json::to_string(&value).ok()
}

async fn copy_if_present(src: &Path, dst: &Path) {
    if let Ok(content) = tokio::fs::read(src).await {
        let _ = tokio::fs::write(dst, content).await;
    }
}

fn is_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

pub(crate) fn is_safe_subpath(subpath: &str) -> bool {
    if subpath.is_empty()
        || subpath.starts_with('/')
        || subpath.starts_with('\\')
        || subpath.contains('\0')
        || subpath.contains(':')
    {
        return false;
    }
    let path = Path::new(subpath);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}
