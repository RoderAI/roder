use crate::error::{ClaudeSDKError, Result};
use crate::session_store::{
    project_key_for_directory, SessionKey, SessionStoreEntry, SessionStoreHandle,
};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

const MAX_PENDING_ENTRIES: usize = 500;
const MAX_PENDING_BYTES: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct ImportSessionOptions {
    pub directory: Option<String>,
    pub include_subagents: bool,
    pub batch_size: usize,
}

impl Default for ImportSessionOptions {
    fn default() -> Self {
        Self {
            directory: None,
            include_subagents: true,
            batch_size: MAX_PENDING_ENTRIES,
        }
    }
}

pub async fn import_session_to_store(
    session_id: &str,
    store: &SessionStoreHandle,
    options: ImportSessionOptions,
) -> Result<()> {
    validate_uuid(session_id)?;
    let resolved = resolve_session_file_path(session_id, options.directory.as_deref())
        .ok_or_else(|| ClaudeSDKError::Session(format!("Session {session_id} not found")))?;
    let project_key = resolved
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| ClaudeSDKError::Session("session path has no project key".to_string()))?
        .to_string();
    let batch_size = if options.batch_size == 0 {
        MAX_PENDING_ENTRIES
    } else {
        options.batch_size
    };

    let main_key = SessionKey {
        project_key: project_key.clone(),
        session_id: session_id.to_string(),
        subpath: None,
    };
    append_jsonl_file_in_batches(&resolved, main_key, store, batch_size).await?;

    if options.include_subagents {
        import_subagents(&resolved, &project_key, session_id, store, batch_size).await?;
    }
    Ok(())
}

async fn import_subagents(
    session_file: &Path,
    project_key: &str,
    session_id: &str,
    store: &SessionStoreHandle,
    batch_size: usize,
) -> Result<()> {
    let session_dir = session_file.with_extension("");
    let subagents_dir = session_dir.join("subagents");
    for file_path in collect_jsonl_files(&subagents_dir) {
        let subpath = subpath_for_file(&session_dir, &file_path)?;
        let key = SessionKey {
            project_key: project_key.to_string(),
            session_id: session_id.to_string(),
            subpath: Some(subpath),
        };
        append_jsonl_file_in_batches(&file_path, key.clone(), store, batch_size).await?;
        if let Some(meta) = read_meta_sidecar(&file_path).await? {
            store.append(key, vec![meta]).await?;
        }
    }
    Ok(())
}

async fn append_jsonl_file_in_batches(
    file_path: &Path,
    key: SessionKey,
    store: &SessionStoreHandle,
    batch_size: usize,
) -> Result<()> {
    let file = tokio::fs::File::open(file_path).await?;
    let mut lines = BufReader::new(file).lines();
    let mut batch = Vec::new();
    let mut nbytes = 0usize;

    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }
        nbytes += line.len();
        let entry = serde_json::from_str::<SessionStoreEntry>(&line)?;
        batch.push(entry);
        if batch.len() >= batch_size || nbytes >= MAX_PENDING_BYTES {
            store
                .append(key.clone(), std::mem::take(&mut batch))
                .await?;
            nbytes = 0;
        }
    }
    if !batch.is_empty() {
        store.append(key, batch).await?;
    }
    Ok(())
}

async fn read_meta_sidecar(file_path: &Path) -> Result<Option<SessionStoreEntry>> {
    let meta_path = file_path.with_file_name(format!(
        "{}.meta.json",
        file_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
    ));
    let content = match tokio::fs::read_to_string(&meta_path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut meta = serde_json::from_str::<SessionStoreEntry>(&content)?;
    let mut entry = SessionStoreEntry::new();
    entry.insert(
        "type".to_string(),
        serde_json::Value::String("agent_metadata".to_string()),
    );
    entry.append(&mut meta);
    Ok(Some(entry))
}

fn resolve_session_file_path(session_id: &str, directory: Option<&str>) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    if let Some(directory) = directory {
        let canonical = canonicalize_path(Path::new(directory));
        if let Some(found) =
            find_project_dir(&canonical).and_then(|dir| stat_candidate(&dir, &file_name))
        {
            return Some(found);
        }
        return None;
    }

    let projects_dir = projects_dir();
    let entries = std::fs::read_dir(projects_dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().ok().is_some_and(|ty| ty.is_dir()) {
            if let Some(found) = stat_candidate(&entry.path(), &file_name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_project_dir(project_path: &Path) -> Option<PathBuf> {
    let exact = projects_dir().join(project_key_for_directory(Some(project_path)));
    if exact.is_dir() {
        Some(exact)
    } else {
        None
    }
}

fn stat_candidate(project_dir: &Path, file_name: &str) -> Option<PathBuf> {
    let candidate = project_dir.join(file_name);
    match std::fs::metadata(&candidate) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Some(candidate),
        _ => None,
    }
}

fn collect_jsonl_files(base_dir: &Path) -> Vec<PathBuf> {
    let mut output = Vec::new();
    collect_jsonl_files_inner(base_dir, &mut output);
    output
}

fn collect_jsonl_files_inner(base_dir: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_inner(&path, output);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
}

fn subpath_for_file(session_dir: &Path, file_path: &Path) -> Result<String> {
    let relative = file_path.strip_prefix(session_dir).map_err(|_| {
        ClaudeSDKError::Session("subagent path escaped session directory".to_string())
    })?;
    let mut parts = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(last) = parts.last_mut() {
        if let Some(stripped) = last.strip_suffix(".jsonl") {
            *last = stripped.to_string();
        }
    }
    Ok(parts.join("/"))
}

fn projects_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .unwrap_or_else(|| PathBuf::from(".claude"))
        .join("projects")
}

fn canonicalize_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

fn validate_uuid(session_id: &str) -> Result<()> {
    uuid::Uuid::parse_str(session_id)
        .map(|_| ())
        .map_err(|_| ClaudeSDKError::Session(format!("Invalid session_id: {session_id}")))
}
