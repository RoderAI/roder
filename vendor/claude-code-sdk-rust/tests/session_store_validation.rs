use async_trait::async_trait;
use claude_code_sdk_rust::internal::session_store_validation::validate_session_store_options;
use claude_code_sdk_rust::{
    list_sessions_from_store, ClaudeAgentClient, ClaudeAgentOptions, InMemorySessionStore,
    SessionKey, SessionStore, SessionStoreEntry, SessionStoreHandle, SessionStoreListEntry,
};

#[derive(Clone)]
struct MinimalStore;

#[derive(Clone)]
struct FlakyListStore {
    good_session_id: String,
    bad_session_id: String,
}

#[async_trait]
impl SessionStore for MinimalStore {
    async fn append(
        &self,
        _key: SessionKey,
        _entries: Vec<SessionStoreEntry>,
    ) -> claude_code_sdk_rust::Result<()> {
        Ok(())
    }

    async fn load(
        &self,
        _key: SessionKey,
    ) -> claude_code_sdk_rust::Result<Option<Vec<SessionStoreEntry>>> {
        Ok(None)
    }
}

#[async_trait]
impl SessionStore for FlakyListStore {
    fn supports_list_sessions(&self) -> bool {
        true
    }

    async fn append(
        &self,
        _key: SessionKey,
        _entries: Vec<SessionStoreEntry>,
    ) -> claude_code_sdk_rust::Result<()> {
        Ok(())
    }

    async fn load(
        &self,
        key: SessionKey,
    ) -> claude_code_sdk_rust::Result<Option<Vec<SessionStoreEntry>>> {
        if key.session_id == self.bad_session_id {
            return Err(claude_code_sdk_rust::ClaudeSDKError::Session(
                "backend down".to_string(),
            ));
        }
        Ok(Some(vec![serde_json::json!({
            "type": "user",
            "uuid": uuid::Uuid::new_v4().to_string(),
            "session_id": self.good_session_id,
            "timestamp": "2026-05-09T10:00:00Z",
            "message": {"role": "user", "content": "good prompt"}
        })
        .as_object()
        .unwrap()
        .clone()]))
    }

    async fn list_sessions(
        &self,
        _project_key: &str,
    ) -> claude_code_sdk_rust::Result<Vec<SessionStoreListEntry>> {
        Ok(vec![
            SessionStoreListEntry {
                session_id: self.good_session_id.clone(),
                mtime: 20,
            },
            SessionStoreListEntry {
                session_id: self.bad_session_id.clone(),
                mtime: 10,
            },
        ])
    }
}

#[test]
fn session_store_options_accept_store_without_continue() {
    let options = ClaudeAgentOptions::builder()
        .session_store(MinimalStore)
        .build();

    validate_session_store_options(&options).expect("minimal store is valid without continue");
}

#[test]
fn session_store_continue_requires_list_sessions_when_resume_is_absent() {
    let options = ClaudeAgentOptions::builder()
        .session_store(MinimalStore)
        .continue_conversation(true)
        .build();

    let err = validate_session_store_options(&options).expect_err("continue should fail fast");
    assert!(err.to_string().contains("list_sessions"));
}

#[test]
fn session_store_continue_allows_explicit_resume_without_list_sessions() {
    let options = ClaudeAgentOptions::builder()
        .session_store(MinimalStore)
        .continue_conversation(true)
        .resume("00000000-0000-4000-8000-000000000000")
        .build();

    validate_session_store_options(&options).expect("resume wins over continue");
}

#[test]
fn session_store_continue_accepts_in_memory_store() {
    let options = ClaudeAgentOptions::builder()
        .session_store(InMemorySessionStore::new())
        .continue_conversation(true)
        .build();

    validate_session_store_options(&options).expect("in-memory store supports list_sessions");
}

#[test]
fn session_store_rejects_file_checkpointing_combo() {
    let options = ClaudeAgentOptions::builder()
        .session_store(InMemorySessionStore::new())
        .enable_file_checkpointing(true)
        .build();

    let err = validate_session_store_options(&options).expect_err("checkpointing should fail fast");
    assert!(err.to_string().contains("enable_file_checkpointing"));
}

#[test]
fn client_new_validates_session_store_options_before_spawn() {
    let options = ClaudeAgentOptions::builder()
        .session_store(MinimalStore)
        .continue_conversation(true)
        .build();

    let err = ClaudeAgentClient::new(options).expect_err("client construction should fail fast");
    assert!(err.to_string().contains("list_sessions"));
}

#[tokio::test]
async fn list_sessions_from_store_requires_list_sessions_support() {
    let handle = SessionStoreHandle::new(MinimalStore);

    let err = list_sessions_from_store(&handle, None, None, 0)
        .await
        .expect_err("list_sessions_from_store should require list_sessions");
    assert!(err.to_string().contains("list_sessions"));
}

#[tokio::test]
async fn list_sessions_from_store_degrades_rows_when_load_fails() {
    let good_session_id = uuid::Uuid::new_v4().to_string();
    let bad_session_id = uuid::Uuid::new_v4().to_string();
    let handle = SessionStoreHandle::new(FlakyListStore {
        good_session_id: good_session_id.clone(),
        bad_session_id: bad_session_id.clone(),
    });

    let sessions = list_sessions_from_store(&handle, None, None, 0)
        .await
        .expect("one bad row should not fail the whole list");

    let good = sessions
        .iter()
        .find(|session| session.session_id == good_session_id)
        .expect("good row");
    let bad = sessions
        .iter()
        .find(|session| session.session_id == bad_session_id)
        .expect("degraded bad row");
    assert_eq!(good.summary, "good prompt");
    assert_eq!(bad.summary, "");
    assert_eq!(bad.last_modified, 10);
}
