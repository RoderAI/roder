use claude_code_sdk_rust::session_store::{
    project_key_for_directory, InMemorySessionStore, SessionKey, SessionStore,
};
use claude_code_sdk_rust::sessions::{
    delete_session_via_store, fork_session_via_store, get_session_info_from_store,
    get_session_messages_from_store, get_subagent_messages_from_store, list_sessions_from_store,
    list_subagents_from_store, rename_session_via_store, tag_session_via_store,
};

fn entry(
    message_type: &str,
    session_id: &str,
    content: &str,
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": message_type,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "timestamp": "2026-05-08T10:00:00Z",
        "session_id": session_id,
        "cwd": "/repo",
        "message": {
            "role": message_type,
            "content": content
        }
    })
    .as_object()
    .unwrap()
    .clone()
}

#[tokio::test]
async fn lists_sessions_from_store_using_summary_sidecars() {
    let cwd = std::env::temp_dir().join(format!("claude-rust-store-list-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let store = InMemorySessionStore::new();
    let older = uuid::Uuid::new_v4().to_string();
    let newer = uuid::Uuid::new_v4().to_string();

    store
        .append(
            SessionKey {
                project_key: project_key.clone(),
                session_id: older.clone(),
                subpath: None,
            },
            vec![entry("user", &older, "older prompt")],
        )
        .await
        .unwrap();
    store
        .append(
            SessionKey {
                project_key,
                session_id: newer.clone(),
                subpath: None,
            },
            vec![entry("user", &newer, "newer prompt")],
        )
        .await
        .unwrap();
    let handle = claude_code_sdk_rust::session_store::SessionStoreHandle::new(store);

    let sessions = list_sessions_from_store(&handle, Some(cwd.to_str().unwrap()), None, 0)
        .await
        .unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id, newer);
    assert_eq!(sessions[0].summary, "newer prompt");
    assert_eq!(sessions[1].session_id, older);

    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn gets_info_and_messages_from_store() {
    let cwd = std::env::temp_dir().join(format!(
        "claude-rust-store-messages-{}",
        uuid::Uuid::new_v4()
    ));
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
            vec![
                entry("user", &session_id, "hello"),
                entry("assistant", &session_id, "hi"),
                serde_json::json!({"type": "tag", "tag": "important"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ],
        )
        .await
        .unwrap();
    let handle = claude_code_sdk_rust::session_store::SessionStoreHandle::new(store);

    let info = get_session_info_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap()
        .expect("info");
    assert_eq!(info.summary, "hello");
    assert_eq!(info.tag.as_deref(), Some("important"));

    let messages =
        get_session_messages_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()), None, 0)
            .await
            .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].r#type, "user");
    assert_eq!(messages[1].r#type, "assistant");

    let messages = get_session_messages_from_store(
        &handle,
        &session_id,
        Some(cwd.to_str().unwrap()),
        Some(0),
        0,
    )
    .await
    .unwrap();
    assert_eq!(messages.len(), 2);

    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn invalid_store_session_id_returns_empty_results() {
    let handle =
        claude_code_sdk_rust::session_store::SessionStoreHandle::new(InMemorySessionStore::new());

    assert!(
        get_session_info_from_store(&handle, "../../etc/passwd", None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_session_messages_from_store(&handle, "../../etc/passwd", None, None, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn mutates_store_session_metadata_and_delete() {
    let cwd =
        std::env::temp_dir().join(format!("claude-rust-store-mutate-{}", uuid::Uuid::new_v4()));
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
            vec![entry("user", &session_id, "original")],
        )
        .await
        .unwrap();
    let handle = claude_code_sdk_rust::session_store::SessionStoreHandle::new(store);

    rename_session_via_store(
        &handle,
        &session_id,
        "  renamed  ",
        Some(cwd.to_str().unwrap()),
    )
    .await
    .unwrap();
    tag_session_via_store(
        &handle,
        &session_id,
        Some(" important "),
        Some(cwd.to_str().unwrap()),
    )
    .await
    .unwrap();

    let info = get_session_info_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.summary, "renamed");
    assert_eq!(info.custom_title.as_deref(), Some("renamed"));
    assert_eq!(info.tag.as_deref(), Some("important"));

    tag_session_via_store(&handle, &session_id, None, Some(cwd.to_str().unwrap()))
        .await
        .unwrap();
    let info = get_session_info_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.tag, None);

    delete_session_via_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap();
    assert!(
        get_session_info_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
            .await
            .unwrap()
            .is_none()
    );

    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn lists_and_reads_subagent_messages_from_store() {
    let cwd = std::env::temp_dir().join(format!(
        "claude-rust-store-subagents-{}",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let session_id = uuid::Uuid::new_v4().to_string();
    let store = InMemorySessionStore::new();
    store
        .append(
            SessionKey {
                project_key: project_key.clone(),
                session_id: session_id.clone(),
                subpath: Some("subagents/workflows/run-1/agent-alpha".to_string()),
            },
            vec![
                entry("user", &session_id, "sub hello"),
                serde_json::json!({"type": "agent_metadata", "name": "alpha"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ],
        )
        .await
        .unwrap();
    store
        .append(
            SessionKey {
                project_key,
                session_id: session_id.clone(),
                subpath: Some("metadata/ignored".to_string()),
            },
            vec![entry("user", &session_id, "ignored")],
        )
        .await
        .unwrap();
    let handle = claude_code_sdk_rust::session_store::SessionStoreHandle::new(store);

    let subagents = list_subagents_from_store(&handle, &session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap();
    assert_eq!(subagents, vec!["alpha".to_string()]);

    let messages = get_subagent_messages_from_store(
        &handle,
        &session_id,
        "alpha",
        Some(cwd.to_str().unwrap()),
        None,
        0,
    )
    .await
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].message["content"], "sub hello");

    let _ = tokio::fs::remove_dir_all(cwd).await;
}

#[tokio::test]
async fn forks_store_session_with_new_session_and_message_ids() {
    let cwd = std::env::temp_dir().join(format!("claude-rust-store-fork-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let project_key = project_key_for_directory(Some(&cwd));
    let session_id = uuid::Uuid::new_v4().to_string();
    let store = InMemorySessionStore::new();
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    let mut first_entry = entry("user", &session_id, "first");
    first_entry.insert("uuid".to_string(), serde_json::json!(first));
    let mut second_entry = entry("assistant", &session_id, "second");
    second_entry.insert("uuid".to_string(), serde_json::json!(second));
    store
        .append(
            SessionKey {
                project_key,
                session_id: session_id.clone(),
                subpath: None,
            },
            vec![first_entry, second_entry],
        )
        .await
        .unwrap();
    let handle = claude_code_sdk_rust::session_store::SessionStoreHandle::new(store);

    let fork = fork_session_via_store(
        &handle,
        &session_id,
        Some(cwd.to_str().unwrap()),
        None,
        Some("fork title"),
    )
    .await
    .unwrap();

    assert_ne!(fork.session_id, session_id);
    let fork_messages = get_session_messages_from_store(
        &handle,
        &fork.session_id,
        Some(cwd.to_str().unwrap()),
        None,
        0,
    )
    .await
    .unwrap();
    assert_eq!(fork_messages.len(), 2);
    assert_ne!(fork_messages[0].uuid, first);
    assert_eq!(fork_messages[0].session_id, fork.session_id);

    let info = get_session_info_from_store(&handle, &fork.session_id, Some(cwd.to_str().unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.custom_title.as_deref(), Some("fork title"));

    let _ = tokio::fs::remove_dir_all(cwd).await;
}
