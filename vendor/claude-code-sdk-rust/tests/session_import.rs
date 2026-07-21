use claude_code_sdk_rust::{
    import_session_to_store, project_key_for_directory, ImportSessionOptions, InMemorySessionStore,
    SessionKey, SessionStore, SessionStoreHandle,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

const SESSION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn unique_tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "claude-agent-sdk-rust-{name}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn entry(i: usize) -> serde_json::Map<String, serde_json::Value> {
    let mut entry = serde_json::Map::new();
    entry.insert("type".to_string(), json!("user"));
    entry.insert("uuid".to_string(), json!(format!("u{i}")));
    entry.insert(
        "timestamp".to_string(),
        json!(format!("2026-01-01T00:00:{i:02}Z")),
    );
    entry
}

fn write_jsonl(path: &Path, entries: &[serde_json::Map<String, serde_json::Value>]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut content = String::new();
    for entry in entries {
        content.push_str(&serde_json::to_string(entry).unwrap());
        content.push('\n');
    }
    std::fs::write(path, content).unwrap();
}

struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var(self.key, old);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

async fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

async fn setup(name: &str) -> (MutexGuard<'static, ()>, PathBuf, PathBuf, PathBuf, EnvGuard) {
    let lock = env_lock().await;
    let root = unique_tmp(name);
    let cwd = root.join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let config = root.join("claude_config");
    let guard = EnvGuard::set("CLAUDE_CONFIG_DIR", &config);
    let project_key = project_key_for_directory(Some(&cwd));
    let claude_dir = config.join("projects").join(project_key);
    std::fs::create_dir_all(&claude_dir).unwrap();
    (lock, root, cwd, claude_dir, guard)
}

#[tokio::test]
async fn imports_main_transcript() {
    let (_lock, _root, cwd, claude_dir, _guard) = setup("main").await;
    let entries = (0..7).map(entry).collect::<Vec<_>>();
    write_jsonl(&claude_dir.join(format!("{SESSION_ID}.jsonl")), &entries);

    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store.clone());
    import_session_to_store(
        SESSION_ID,
        &handle,
        ImportSessionOptions {
            directory: Some(cwd.to_string_lossy().to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let key = SessionKey {
        project_key: project_key_for_directory(Some(&cwd)),
        session_id: SESSION_ID.to_string(),
        subpath: None,
    };
    assert_eq!(store.get_entries(key).await, entries);
}

#[tokio::test]
async fn skips_blank_lines_and_defaults_nonpositive_batch_size() {
    let (_lock, _root, cwd, claude_dir, _guard) = setup("blank").await;
    let path = claude_dir.join(format!("{SESSION_ID}.jsonl"));
    std::fs::write(
        &path,
        format!(
            "{}\n\n{}\n",
            serde_json::to_string(&entry(0)).unwrap(),
            serde_json::to_string(&entry(1)).unwrap()
        ),
    )
    .unwrap();

    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store.clone());
    import_session_to_store(
        SESSION_ID,
        &handle,
        ImportSessionOptions {
            directory: Some(cwd.to_string_lossy().to_string()),
            batch_size: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let key = SessionKey {
        project_key: project_key_for_directory(Some(&cwd)),
        session_id: SESSION_ID.to_string(),
        subpath: None,
    };
    assert_eq!(store.get_entries(key).await, vec![entry(0), entry(1)]);
}

#[tokio::test]
async fn imports_subagents_and_meta_sidecars() {
    let (_lock, _root, cwd, claude_dir, _guard) = setup("subagents").await;
    write_jsonl(&claude_dir.join(format!("{SESSION_ID}.jsonl")), &[entry(0)]);
    let sub_file = claude_dir
        .join(SESSION_ID)
        .join("subagents")
        .join("workflows")
        .join("run-1")
        .join("agent-abc.jsonl");
    write_jsonl(&sub_file, &[entry(10)]);
    std::fs::write(
        sub_file.with_file_name("agent-abc.meta.json"),
        r#"{"agentType":"coder","worktreePath":"/tmp/wt"}"#,
    )
    .unwrap();

    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store.clone());
    import_session_to_store(
        SESSION_ID,
        &handle,
        ImportSessionOptions {
            directory: Some(cwd.to_string_lossy().to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let sub_key = SessionKey {
        project_key: project_key_for_directory(Some(&cwd)),
        session_id: SESSION_ID.to_string(),
        subpath: Some("subagents/workflows/run-1/agent-abc".to_string()),
    };
    let stored = store.get_entries(sub_key.clone()).await;
    assert_eq!(stored[0], entry(10));
    assert_eq!(
        stored[1],
        serde_json::Map::from_iter([
            ("type".to_string(), json!("agent_metadata")),
            ("agentType".to_string(), json!("coder")),
            ("worktreePath".to_string(), json!("/tmp/wt")),
        ])
    );
    assert_eq!(
        store
            .list_subkeys(claude_code_sdk_rust::SessionListSubkeysKey {
                project_key: project_key_for_directory(Some(&cwd)),
                session_id: SESSION_ID.to_string(),
            })
            .await
            .unwrap(),
        vec!["subagents/workflows/run-1/agent-abc".to_string()]
    );
}

#[tokio::test]
async fn include_subagents_false_skips_subagents() {
    let (_lock, _root, cwd, claude_dir, _guard) = setup("skip-subagents").await;
    write_jsonl(&claude_dir.join(format!("{SESSION_ID}.jsonl")), &[entry(0)]);
    write_jsonl(
        &claude_dir
            .join(SESSION_ID)
            .join("subagents")
            .join("agent-abc.jsonl"),
        &[entry(10)],
    );

    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store.clone());
    import_session_to_store(
        SESSION_ID,
        &handle,
        ImportSessionOptions {
            directory: Some(cwd.to_string_lossy().to_string()),
            include_subagents: false,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(store
        .list_subkeys(claude_code_sdk_rust::SessionListSubkeysKey {
            project_key: project_key_for_directory(Some(&cwd)),
            session_id: SESSION_ID.to_string(),
        })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn validates_uuid_and_reports_missing_session() {
    let (_lock, _root, cwd, _claude_dir, _guard) = setup("validation").await;
    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store);

    let invalid = import_session_to_store("../../etc/passwd", &handle, Default::default())
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("Invalid session_id"));

    let missing = import_session_to_store(
        SESSION_ID,
        &handle,
        ImportSessionOptions {
            directory: Some(cwd.to_string_lossy().to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(missing.to_string().contains("not found"));
}

#[tokio::test]
async fn directory_none_keys_from_resolved_path_not_cwd() {
    let (_lock, root, cwd, claude_dir, _guard) = setup("directory-none").await;
    write_jsonl(&claude_dir.join(format!("{SESSION_ID}.jsonl")), &[entry(0)]);

    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&elsewhere).unwrap();

    let store = InMemorySessionStore::new();
    let handle = SessionStoreHandle::new(store.clone());
    let result = import_session_to_store(SESSION_ID, &handle, Default::default()).await;
    std::env::set_current_dir(old_cwd).unwrap();
    result.unwrap();

    let key = SessionKey {
        project_key: project_key_for_directory(Some(&cwd)),
        session_id: SESSION_ID.to_string(),
        subpath: None,
    };
    assert_eq!(store.get_entries(key).await, vec![entry(0)]);
}
