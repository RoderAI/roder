use super::session_resume::{
    apply_materialized_options, is_safe_subpath, materialize_resume_session,
};
use crate::session_store::{
    project_key_for_directory, InMemorySessionStore, SessionKey, SessionStore, SessionStoreEntry,
};
use crate::types::ClaudeAgentOptions;

fn entry(content: &str) -> SessionStoreEntry {
    serde_json::json!({
        "type": "user",
        "uuid": uuid::Uuid::new_v4().to_string(),
        "message": {"content": content},
    })
    .as_object()
    .unwrap()
    .clone()
}

#[tokio::test]
async fn materializes_explicit_resume_to_temp_config_dir() {
    let cwd = std::env::temp_dir().join(format!("claude-rust-resume-cwd-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let session_id = uuid::Uuid::new_v4().to_string();
    let store = InMemorySessionStore::new();
    store
        .append(
            SessionKey {
                project_key,
                session_id: session_id.clone(),
                subpath: None,
            },
            vec![entry("hello")],
        )
        .await
        .unwrap();
    let options = ClaudeAgentOptions::builder()
        .cwd(cwd.to_string_lossy().to_string())
        .session_store(store)
        .resume(session_id.clone())
        .build();

    let materialized = materialize_resume_session(&options)
        .await
        .unwrap()
        .expect("materialized");

    let jsonl = tokio::fs::read_to_string(
        materialized
            .config_dir
            .join("projects")
            .join(project_key_for_directory(Some(&cwd)))
            .join(format!("{session_id}.jsonl")),
    )
    .await
    .unwrap();
    assert!(jsonl.contains("hello"));

    let applied = apply_materialized_options(&options, &materialized);
    assert_eq!(applied.resume.as_deref(), Some(session_id.as_str()));
    assert!(!applied.continue_conversation);
    assert_eq!(
        applied.env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
        Some(materialized.config_dir.to_string_lossy().as_ref())
    );

    materialized.cleanup().await;
    assert!(!materialized.config_dir.exists());
    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn materializes_continue_from_newest_non_sidechain_session() {
    let cwd =
        std::env::temp_dir().join(format!("claude-rust-continue-cwd-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let sidechain_id = uuid::Uuid::new_v4().to_string();
    let main_id = uuid::Uuid::new_v4().to_string();
    let store = InMemorySessionStore::new();
    let mut sidechain = entry("sidechain");
    sidechain.insert("isSidechain".to_string(), serde_json::Value::Bool(true));
    store
        .append(
            SessionKey {
                project_key: project_key.clone(),
                session_id: main_id.clone(),
                subpath: None,
            },
            vec![entry("main")],
        )
        .await
        .unwrap();
    store
        .append(
            SessionKey {
                project_key,
                session_id: sidechain_id,
                subpath: None,
            },
            vec![sidechain],
        )
        .await
        .unwrap();
    let options = ClaudeAgentOptions::builder()
        .cwd(cwd.to_string_lossy().to_string())
        .session_store(store)
        .continue_conversation(true)
        .build();

    let materialized = materialize_resume_session(&options)
        .await
        .unwrap()
        .expect("materialized");

    assert_eq!(materialized.resume_session_id, main_id);
    materialized.cleanup().await;
    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn materializes_safe_subkeys_and_metadata_sidecars() {
    let cwd = std::env::temp_dir().join(format!("claude-rust-subkey-cwd-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let session_id = uuid::Uuid::new_v4().to_string();
    let store = InMemorySessionStore::new();
    store
        .append(
            SessionKey {
                project_key: project_key.clone(),
                session_id: session_id.clone(),
                subpath: None,
            },
            vec![entry("main")],
        )
        .await
        .unwrap();
    store
        .append(
            SessionKey {
                project_key: project_key.clone(),
                session_id: session_id.clone(),
                subpath: Some("subagents/agent-1".to_string()),
            },
            vec![
                entry("subagent"),
                serde_json::json!({"type": "agent_metadata", "name": "reviewer"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ],
        )
        .await
        .unwrap();
    let options = ClaudeAgentOptions::builder()
        .cwd(cwd.to_string_lossy().to_string())
        .session_store(store)
        .resume(session_id.clone())
        .build();

    let materialized = materialize_resume_session(&options)
        .await
        .unwrap()
        .expect("materialized");
    let session_dir = materialized
        .config_dir
        .join("projects")
        .join(project_key)
        .join(session_id);

    let transcript = tokio::fs::read_to_string(session_dir.join("subagents/agent-1.jsonl"))
        .await
        .unwrap();
    assert!(transcript.contains("subagent"));
    let metadata = tokio::fs::read_to_string(session_dir.join("subagents/agent-1.meta.json"))
        .await
        .unwrap();
    assert!(metadata.contains("reviewer"));
    assert!(!metadata.contains("agent_metadata"));

    materialized.cleanup().await;
    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn rejects_invalid_resume_id_without_materializing() {
    let options = ClaudeAgentOptions::builder()
        .session_store(InMemorySessionStore::new())
        .resume("../../etc/passwd")
        .build();

    assert!(materialize_resume_session(&options)
        .await
        .unwrap()
        .is_none());
}

#[test]
fn validates_subpaths() {
    assert!(is_safe_subpath("subagents/agent-1"));
    assert!(!is_safe_subpath(""));
    assert!(!is_safe_subpath("../escape"));
    assert!(!is_safe_subpath("/absolute"));
    assert!(!is_safe_subpath("C:\\escape"));
}
