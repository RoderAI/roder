use claude_code_sdk_rust::sessions::{
    delete_session, fork_session, get_session_info, get_session_messages, get_subagent_messages,
    list_sessions, list_subagents, rename_session, tag_session, ListSessionsOptions,
    SessionMutationOptions, SessionQueryOptions,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn unique_tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "claude-agent-sdk-rust-local-sessions-{name}-{}",
        uuid::Uuid::new_v4()
    ))
}

async fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

struct EnvGuard {
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(config: &Path) -> Self {
        let old = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", config);
        Self { old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var("CLAUDE_CONFIG_DIR", old);
        } else {
            std::env::remove_var("CLAUDE_CONFIG_DIR");
        }
    }
}

fn write_jsonl(path: &Path, entries: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut content = String::new();
    for entry in entries {
        content.push_str(&serde_json::to_string(entry).unwrap());
        content.push('\n');
    }
    std::fs::write(path, content).unwrap();
}

fn query_opts(session_id: &str) -> SessionQueryOptions {
    SessionQueryOptions {
        session_id: session_id.to_string(),
        include_messages: true,
        directory: None,
        limit: None,
        offset: None,
    }
}

fn mutation_opts(session_id: &str) -> SessionMutationOptions {
    SessionMutationOptions {
        session_id: session_id.to_string(),
        directory: None,
    }
}

fn query_opts_for_dir(session_id: &str, directory: &Path) -> SessionQueryOptions {
    SessionQueryOptions {
        directory: Some(directory.to_string_lossy().to_string()),
        ..query_opts(session_id)
    }
}

#[tokio::test]
async fn local_sessions_list_info_and_messages_from_jsonl() {
    let _lock = env_lock().await;
    let root = unique_tmp("list");
    let config = root.join(".claude");
    let _guard = EnvGuard::set(&config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let transcript = config
        .join("projects")
        .join("-Users-test-project")
        .join(format!("{session_id}.jsonl"));
    write_jsonl(
        &transcript,
        &[
            json!({
                "type": "user",
                "uuid": "u1",
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"role": "user", "content": "What is 2+2?"}
            }),
            json!({
                "type": "assistant",
                "uuid": "a1",
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "4"}]}
            }),
            json!({"type": "summary", "summary": "Math question"}),
        ],
    );

    let sessions = list_sessions(&ListSessionsOptions::default())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session_id);
    assert_eq!(sessions[0].title, "Math question");
    assert_eq!(sessions[0].message_count, 2);

    let info = get_session_info(&sessions[0].id, &query_opts(&sessions[0].id))
        .await
        .unwrap();
    assert_eq!(info.title, "Math question");

    let messages = get_session_messages(&sessions[0].id, &query_opts(&sessions[0].id))
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "What is 2+2?");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "4");
}

#[tokio::test]
async fn local_sessions_skip_hidden_messages_and_support_rename_delete() {
    let _lock = env_lock().await;
    let root = unique_tmp("mutate");
    let config = root.join(".claude");
    let _guard = EnvGuard::set(&config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let project_dir = config.join("projects").join("-Users-test-project");
    let transcript = project_dir.join(format!("{session_id}.jsonl"));
    let sidecar_dir = project_dir.join(&session_id);
    std::fs::create_dir_all(sidecar_dir.join("subagents")).unwrap();
    std::fs::write(sidecar_dir.join("subagents").join("agent-1.jsonl"), "{}\n").unwrap();
    write_jsonl(
        &transcript,
        &[
            json!({
                "type": "user",
                "uuid": "meta",
                "isMeta": true,
                "message": {"content": "hidden"}
            }),
            json!({
                "type": "user",
                "uuid": "visible",
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"content": "Visible prompt"}
            }),
        ],
    );

    rename_session(&session_id, "Renamed", &mutation_opts(&session_id))
        .await
        .unwrap();
    let info = get_session_info(&session_id, &query_opts(&session_id))
        .await
        .unwrap();
    assert_eq!(info.title, "Renamed");

    let messages = get_session_messages(&session_id, &query_opts(&session_id))
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "visible");

    delete_session(&session_id, &mutation_opts(&session_id))
        .await
        .unwrap();
    assert!(!transcript.exists());
    assert!(!sidecar_dir.exists());
}

#[tokio::test]
async fn local_sessions_tag_and_clear_tag_by_appending_metadata() {
    let _lock = env_lock().await;
    let root = unique_tmp("tag");
    let config = root.join(".claude");
    let _guard = EnvGuard::set(&config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let project_dir = config.join("projects").join("-Users-test-project");
    let transcript = project_dir.join(format!("{session_id}.jsonl"));
    write_jsonl(
        &transcript,
        &[json!({
            "type": "user",
            "uuid": "visible",
            "message": {"content": "Visible prompt"}
        })],
    );

    tag_session(
        &session_id,
        Some("  experiment  "),
        &mutation_opts(&session_id),
    )
    .await
    .unwrap();
    tag_session(&session_id, None, &mutation_opts(&session_id))
        .await
        .unwrap();

    let lines = std::fs::read_to_string(&transcript).unwrap();
    let lines = lines.lines().collect::<Vec<_>>();
    let tagged = serde_json::from_str::<serde_json::Value>(lines[1]).unwrap();
    let cleared = serde_json::from_str::<serde_json::Value>(lines[2]).unwrap();
    assert_eq!(tagged["type"], "tag");
    assert_eq!(tagged["tag"], "experiment");
    assert_eq!(tagged["sessionId"], session_id);
    assert_eq!(cleared["type"], "tag");
    assert_eq!(cleared["tag"], "");

    let err = tag_session(&session_id, Some(" \n\t "), &mutation_opts(&session_id))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("tag must be non-empty"));
}

#[tokio::test]
async fn local_sessions_list_and_read_nested_subagents() {
    let _lock = env_lock().await;
    let root = unique_tmp("subagents");
    let config = root.join(".claude");
    let _guard = EnvGuard::set(&config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let project_dir = config.join("projects").join("-Users-test-project");
    let transcript = project_dir.join(format!("{session_id}.jsonl"));
    write_jsonl(
        &transcript,
        &[json!({
            "type": "user",
            "uuid": "root",
            "message": {"content": "parent prompt"}
        })],
    );
    let subagent_file = project_dir
        .join(&session_id)
        .join("subagents")
        .join("workflows")
        .join("run-1")
        .join("agent-deep.jsonl");
    write_jsonl(
        &subagent_file,
        &[
            json!({
                "type": "assistant",
                "uuid": "branch",
                "parentUuid": "missing",
                "message": {"content": "old branch"}
            }),
            json!({
                "type": "user",
                "uuid": "s1",
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"content": "sub prompt"}
            }),
            json!({
                "type": "assistant",
                "uuid": "s2",
                "parentUuid": "s1",
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": [{"type": "text", "text": "sub answer"}]}
            }),
        ],
    );

    let opts = query_opts(&session_id);
    assert_eq!(
        list_subagents(&session_id, &opts).await.unwrap(),
        vec!["deep".to_string()]
    );

    let messages = get_subagent_messages(&session_id, "deep", &opts)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, "s1");
    assert_eq!(messages[0].content, "sub prompt");
    assert_eq!(messages[1].id, "s2");
    assert_eq!(messages[1].content, "sub answer");
    assert!(get_subagent_messages(&session_id, "missing", &opts)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn local_sessions_scope_queries_to_directory_and_page_messages() {
    let _lock = env_lock().await;
    let root = unique_tmp("directory");
    let config = root.join(".claude");
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();
    let _guard = EnvGuard::set(&config);
    let session_a = uuid::Uuid::new_v4().to_string();
    let session_b = uuid::Uuid::new_v4().to_string();
    let project_a_dir =
        config
            .join("projects")
            .join(claude_code_sdk_rust::project_key_for_directory(Some(
                &project_a,
            )));
    let project_b_dir =
        config
            .join("projects")
            .join(claude_code_sdk_rust::project_key_for_directory(Some(
                &project_b,
            )));

    write_jsonl(
        &project_a_dir.join(format!("{session_a}.jsonl")),
        &[
            json!({"type": "user", "uuid": "a1", "message": {"content": "a one"}}),
            json!({"type": "assistant", "uuid": "a2", "message": {"content": "a two"}}),
            json!({"type": "user", "uuid": "a3", "message": {"content": "a three"}}),
        ],
    );
    write_jsonl(
        &project_b_dir.join(format!("{session_b}.jsonl")),
        &[json!({"type": "user", "uuid": "b1", "message": {"content": "b one"}})],
    );

    let sessions = list_sessions(&ListSessionsOptions {
        directory: Some(project_a.to_string_lossy().to_string()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session_a);

    let mut opts = query_opts_for_dir(&session_a, &project_a);
    opts.limit = Some(1);
    opts.offset = Some(1);
    let messages = get_session_messages(&session_a, &opts).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "a2");

    assert!(
        get_session_messages(&session_b, &query_opts_for_dir(&session_b, &project_a))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn local_sessions_fork_remaps_ids_and_supports_slice_title() {
    let _lock = env_lock().await;
    let root = unique_tmp("fork");
    let config = root.join(".claude");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let _guard = EnvGuard::set(&config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let u1 = uuid::Uuid::new_v4().to_string();
    let a1 = uuid::Uuid::new_v4().to_string();
    let u2 = uuid::Uuid::new_v4().to_string();
    let project_dir =
        config
            .join("projects")
            .join(claude_code_sdk_rust::project_key_for_directory(Some(
                &project,
            )));
    write_jsonl(
        &project_dir.join(format!("{session_id}.jsonl")),
        &[
            json!({
                "type": "user",
                "uuid": u1,
                "sessionId": session_id,
                "message": {"content": "one"}
            }),
            json!({
                "type": "assistant",
                "uuid": a1,
                "parentUuid": u1,
                "sessionId": session_id,
                "message": {"content": "two"}
            }),
            json!({
                "type": "user",
                "uuid": u2,
                "parentUuid": a1,
                "sessionId": session_id,
                "message": {"content": "three"}
            }),
        ],
    );

    let fork = fork_session(
        &session_id,
        &SessionMutationOptions {
            session_id: session_id.clone(),
            directory: Some(project.to_string_lossy().to_string()),
        },
        Some(&a1),
        Some("Fork title"),
    )
    .await
    .unwrap();

    assert_ne!(fork.session_id, session_id);
    let fork_path = project_dir.join(format!("{}.jsonl", fork.session_id));
    let lines = std::fs::read_to_string(fork_path).unwrap();
    let entries = lines
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["sessionId"], fork.session_id);
    assert_eq!(entries[1]["sessionId"], fork.session_id);
    assert_eq!(entries[0]["forkedFrom"], session_id);
    assert_ne!(entries[0]["uuid"], u1);
    assert_eq!(entries[1]["parentUuid"], entries[0]["uuid"]);
    assert_eq!(entries[2]["type"], "custom-title");
    assert_eq!(entries[2]["customTitle"], "Fork title");
    assert!(!entries.iter().any(|entry| entry["uuid"] == u2));
}
