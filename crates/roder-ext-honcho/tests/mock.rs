use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use roder_api::memory::{MemoryQuery, MemoryRecord, MemoryScope, MemoryStore};
use roder_ext_honcho::{HonchoMemoryConfig, HonchoMemoryStore};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Stateful fake Honcho covering every endpoint the store uses: workspace /
/// peer / session get-or-create, message add / get / metadata update,
/// workspace search, session message list, and session list. Search is naive
/// term matching; filters apply `metadata` equality the way Honcho's filter
/// language does.
#[derive(Default)]
struct MockHoncho {
    messages: Vec<MockMessage>,
    sessions: HashMap<String, Value>,
    next_id: usize,
    /// Simulates a server that silently drops the `peer_id` filter key (the
    /// failure mode the store's client-side authorship re-checks defend
    /// against).
    ignore_peer_filter: bool,
}

#[derive(Clone)]
struct MockMessage {
    id: String,
    session_id: String,
    peer_id: String,
    content: String,
    metadata: Value,
}

impl MockMessage {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "content": self.content,
            "peer_id": self.peer_id,
            "session_id": self.session_id,
            "workspace_id": "ws-test",
            "metadata": self.metadata,
            "created_at": "2026-01-01T00:00:00Z",
            "token_count": 1,
        })
    }

    fn matches_filters(&self, filters: &Value) -> bool {
        if let Some(peer_filter) = filters.get("peer_id").and_then(Value::as_str)
            && self.peer_id != peer_filter
        {
            return false;
        }
        let Some(metadata_filter) = filters.get("metadata").and_then(Value::as_object) else {
            return true;
        };
        metadata_filter
            .iter()
            .all(|(key, expected)| self.metadata.get(key) == Some(expected))
    }
}

impl MockHoncho {
    fn effective_filters(&self, filters: &Value) -> Value {
        let mut filters = filters.clone();
        if self.ignore_peer_filter
            && let Some(object) = filters.as_object_mut()
        {
            object.remove("peer_id");
        }
        filters
    }

    fn handle(&mut self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (method, parts.as_slice()) {
            ("POST", ["v3", "workspaces"]) => (200, json!({ "id": body["id"] })),
            ("POST", ["v3", "workspaces", _, "peers"]) => (200, json!({ "id": body["id"] })),
            ("POST", ["v3", "workspaces", _, "sessions"]) => {
                let id = body["id"].as_str().unwrap_or_default().to_string();
                self.sessions
                    .entry(id.clone())
                    .or_insert_with(|| body["metadata"].clone());
                (200, json!({ "id": id }))
            }
            ("POST", ["v3", "workspaces", _, "sessions", session, "messages"]) => {
                let session = (*session).to_string();
                if !self.sessions.contains_key(&session) {
                    return (404, json!({ "error": "session not found" }));
                }
                let mut created = Vec::new();
                for message in body["messages"].as_array().cloned().unwrap_or_default() {
                    self.next_id += 1;
                    let stored = MockMessage {
                        id: format!("msg-{}", self.next_id),
                        session_id: session.clone(),
                        peer_id: message["peer_id"].as_str().unwrap_or_default().to_string(),
                        content: message["content"].as_str().unwrap_or_default().to_string(),
                        metadata: message["metadata"].clone(),
                    };
                    created.push(stored.to_json());
                    self.messages.push(stored);
                }
                (200, Value::Array(created))
            }
            (
                "GET",
                [
                    "v3",
                    "workspaces",
                    _,
                    "sessions",
                    session,
                    "messages",
                    message,
                ],
            ) => self
                .messages
                .iter()
                .find(|stored| stored.id == *message && stored.session_id == *session)
                .map(|stored| (200, stored.to_json()))
                .unwrap_or((404, json!({ "error": "message not found" }))),
            (
                "PUT",
                [
                    "v3",
                    "workspaces",
                    _,
                    "sessions",
                    session,
                    "messages",
                    message,
                ],
            ) => {
                let Some(stored) = self
                    .messages
                    .iter_mut()
                    .find(|stored| stored.id == *message && stored.session_id == *session)
                else {
                    return (404, json!({ "error": "message not found" }));
                };
                stored.metadata = body["metadata"].clone();
                (200, stored.to_json())
            }
            ("POST", ["v3", "workspaces", _, "search"]) => {
                let query = body["query"].as_str().unwrap_or_default().to_lowercase();
                let limit = body["limit"].as_u64().unwrap_or(10) as usize;
                let filters = self.effective_filters(&body["filters"]);
                let hits: Vec<Value> = self
                    .messages
                    .iter()
                    .filter(|message| message.matches_filters(&filters))
                    .filter(|message| {
                        query
                            .split_whitespace()
                            .any(|term| message.content.to_lowercase().contains(term))
                    })
                    .take(limit)
                    .map(MockMessage::to_json)
                    .collect();
                (200, Value::Array(hits))
            }
            (
                "POST",
                [
                    "v3",
                    "workspaces",
                    _,
                    "sessions",
                    session,
                    "messages",
                    "list",
                ],
            ) => {
                if !self.sessions.contains_key(*session) {
                    return (404, json!({ "error": "session not found" }));
                }
                let size = body["size"].as_u64().unwrap_or(50) as usize;
                let filters = self.effective_filters(&body["filters"]);
                let items: Vec<Value> = self
                    .messages
                    .iter()
                    .filter(|message| message.session_id == *session)
                    .filter(|message| message.matches_filters(&filters))
                    .take(size)
                    .map(MockMessage::to_json)
                    .collect();
                (
                    200,
                    json!({ "items": items, "page": 1, "size": size, "total": items.len(), "pages": 1 }),
                )
            }
            ("POST", ["v3", "workspaces", _, "sessions", "list"]) => {
                let metadata_filter = body["filters"]["metadata"].clone();
                let items: Vec<Value> = self
                    .sessions
                    .iter()
                    .filter(|(_, metadata)| {
                        metadata_filter.as_object().is_none_or(|filter| {
                            filter
                                .iter()
                                .all(|(key, expected)| metadata.get(key) == Some(expected))
                        })
                    })
                    .map(|(id, _)| json!({ "id": id }))
                    .collect();
                (
                    200,
                    json!({ "items": items, "page": 1, "size": 50, "total": items.len(), "pages": 1 }),
                )
            }
            _ => (
                404,
                json!({ "error": format!("missing route {method} {path}") }),
            ),
        }
    }
}

async fn spawn_mock() -> (String, Arc<Mutex<MockHoncho>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(MockHoncho::default()));
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 8192];
            let (method, path, body) = loop {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break (String::new(), String::new(), Value::Null);
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(parsed) = parse_request(&buffer) {
                    break parsed;
                }
            };
            if method.is_empty() {
                continue;
            }
            let (status, payload) = server_state.lock().unwrap().handle(&method, &path, body);
            let body = payload.to_string();
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (base_url, state, server)
}

/// Returns None until the buffer holds the full request (headers plus
/// content-length body).
fn parse_request(buffer: &[u8]) -> Option<(String, String, Value)> {
    let text = String::from_utf8_lossy(buffer);
    let header_end = text.find("\r\n\r\n")?;
    let headers = &text[..header_end];
    let mut first = headers.lines().next().unwrap_or_default().split(' ');
    let method = first.next().unwrap_or_default().to_string();
    let path = first.next().unwrap_or_default().to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let body_bytes = &buffer[header_end + 4..];
    if body_bytes.len() < content_length {
        return None;
    }
    let body = serde_json::from_slice(&body_bytes[..content_length]).unwrap_or(Value::Null);
    Some((method, path, body))
}

fn record(text: &str, scope: MemoryScope) -> MemoryRecord {
    MemoryRecord {
        id: None,
        scope,
        text: text.to_string(),
        content_hash: None,
        metadata: json!({ "origin": "mock-test" }),
        usage: None,
        deleted: false,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
    }
}

fn query(text: &str, scope: Option<MemoryScope>, include_global: bool) -> MemoryQuery {
    MemoryQuery {
        scope,
        text: text.to_string(),
        limit: 5,
        include_global,
        provider_id: None,
        model: None,
    }
}

#[tokio::test]
async fn honcho_store_lifecycle_against_mock_server() {
    let (base_url, _state, server) = spawn_mock().await;
    let store = HonchoMemoryStore::new(HonchoMemoryConfig {
        api_key: "test-key".to_string(),
        base_url,
        workspace_id: "ws-test".to_string(),
        peer_id: "roder-memory".to_string(),
        session_id: None,
    });
    let workspace_scope = MemoryScope::Workspace("w1".to_string());

    let global_id = store
        .put(record(
            "global fact about gravel cycling",
            MemoryScope::Global,
        ))
        .await
        .unwrap();
    let alpha_id = store
        .put(record("alpha deadline is friday", workspace_scope.clone()))
        .await
        .unwrap();
    store
        .put(record("note about lunch", workspace_scope.clone()))
        .await
        .unwrap();
    assert!(global_id.starts_with("roder-memory-global/"));
    assert!(alpha_id.starts_with("roder-memory-workspace-w1/"));

    let fetched = store.get(&alpha_id).await.unwrap().unwrap();
    assert_eq!(fetched.text, "alpha deadline is friday");
    assert_eq!(fetched.scope, workspace_scope);
    assert_eq!(fetched.metadata, json!({ "origin": "mock-test" }));

    let results = store
        .search(query("deadline", Some(workspace_scope.clone()), false))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].record.id.as_deref(), Some(alpha_id.as_str()));
    assert!(results[0].citation.is_some());

    // include_global folds Global-scope hits into a workspace-scoped search.
    let results = store
        .search(query("gravel", Some(workspace_scope.clone()), true))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].record.scope, MemoryScope::Global);

    // Update writes a new message and leaves a supersede pointer behind, so
    // the old id still resolves to the new text.
    let mut updated = record("alpha deadline moved to monday", workspace_scope.clone());
    updated.id = Some(alpha_id.clone());
    let new_id = store.put(updated).await.unwrap();
    assert_ne!(new_id, alpha_id);
    let via_old_id = store.get(&alpha_id).await.unwrap().unwrap();
    assert_eq!(via_old_id.text, "alpha deadline moved to monday");
    assert!(!via_old_id.deleted);
    assert_eq!(via_old_id.id.as_deref(), Some(new_id.as_str()));

    // The superseded message is tombstoned out of search.
    let results = store
        .search(query("deadline", Some(workspace_scope.clone()), false))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].record.id.as_deref(), Some(new_id.as_str()));

    // Soft delete via the old id resolves the chain to the live head.
    store.delete(&alpha_id).await.unwrap();
    let results = store
        .search(query("deadline", Some(workspace_scope.clone()), false))
        .await
        .unwrap();
    assert!(results.is_empty());
    let tombstone = store.get(&new_id).await.unwrap().unwrap();
    assert!(tombstone.deleted);

    let listed = store.list(Some(workspace_scope.clone()), 10).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].text, "note about lunch");

    let all = store.list(None, 10).await.unwrap();
    let texts: Vec<&str> = all.iter().map(|record| record.text.as_str()).collect();
    assert_eq!(all.len(), 2);
    assert!(texts.contains(&"note about lunch"));
    assert!(texts.contains(&"global fact about gravel cycling"));

    // Empty-text search behaves like list and honors include_global.
    let results = store
        .search(query("", Some(workspace_scope), true))
        .await
        .unwrap();
    let texts: Vec<&str> = results
        .iter()
        .map(|result| result.record.text.as_str())
        .collect();
    assert_eq!(results.len(), 2);
    assert!(texts.contains(&"note about lunch"));
    assert!(texts.contains(&"global fact about gravel cycling"));

    server.abort();
}

#[tokio::test]
async fn peers_sharing_a_workspace_do_not_read_each_other() {
    let (base_url, _state, server) = spawn_mock().await;
    let config = |peer: &str| HonchoMemoryConfig {
        api_key: "test-key".to_string(),
        base_url: base_url.clone(),
        workspace_id: "ws-test".to_string(),
        peer_id: peer.to_string(),
        session_id: None,
    };
    let store_a = HonchoMemoryStore::new(config("peer-a"));
    let store_b = HonchoMemoryStore::new(config("peer-b"));
    let scope = MemoryScope::Project("project".to_string());

    let a_id = store_a
        .put(record("peer a launch checklist", scope.clone()))
        .await
        .unwrap();

    // Same workspace, same scope-derived session: peer-b must see nothing.
    let results = store_b
        .search(query("launch", Some(scope.clone()), false))
        .await
        .unwrap();
    assert!(results.is_empty());
    assert!(
        store_b
            .list(Some(scope.clone()), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store_b.get(&a_id).await.unwrap().is_none());

    // peer-a still reads its own record through every path.
    let results = store_a
        .search(query("launch", Some(scope.clone()), false))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(store_a.list(Some(scope), 10).await.unwrap().len(), 1);
    assert!(store_a.get(&a_id).await.unwrap().is_some());

    server.abort();
}

/// The client-side authorship re-checks must hold on their own: the server
/// here drops the `peer_id` filter (as a search API that ignores unknown
/// filter keys would), so any cross-peer leak below means a read path is
/// trusting the server-side filter alone.
#[tokio::test]
async fn peer_isolation_holds_when_server_ignores_peer_filter() {
    let (base_url, state, server) = spawn_mock().await;
    state.lock().unwrap().ignore_peer_filter = true;
    let config = |peer: &str| HonchoMemoryConfig {
        api_key: "test-key".to_string(),
        base_url: base_url.clone(),
        workspace_id: "ws-test".to_string(),
        peer_id: peer.to_string(),
        session_id: None,
    };
    let store_a = HonchoMemoryStore::new(config("peer-a"));
    let store_b = HonchoMemoryStore::new(config("peer-b"));
    let scope = MemoryScope::Project("project".to_string());

    let a_id = store_a
        .put(record("peer a launch checklist", scope.clone()))
        .await
        .unwrap();

    // search (non-empty text), list, empty-text search, and get must all
    // come back empty for peer-b despite the broken server-side filter.
    assert!(
        store_b
            .search(query("launch", Some(scope.clone()), false))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store_b
            .list(Some(scope.clone()), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store_b.list(None, 10).await.unwrap().is_empty());
    assert!(
        store_b
            .search(query("", Some(scope.clone()), true))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store_b.get(&a_id).await.unwrap().is_none());

    // peer-a still reads its own record (the client-side check matches).
    assert_eq!(
        store_a
            .search(query("launch", Some(scope.clone()), false))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store_a.list(Some(scope), 10).await.unwrap().len(), 1);

    server.abort();
}

#[tokio::test]
async fn pinned_session_routes_all_scopes_to_one_session() {
    let (base_url, state, server) = spawn_mock().await;
    let store = HonchoMemoryStore::new(HonchoMemoryConfig {
        api_key: "test-key".to_string(),
        base_url,
        workspace_id: "ws-test".to_string(),
        peer_id: "roder-memory".to_string(),
        session_id: Some("pinned-session".to_string()),
    });

    let global_id = store
        .put(record("pinned global", MemoryScope::Global))
        .await
        .unwrap();
    let thread_id = store
        .put(record(
            "pinned thread",
            MemoryScope::Thread("t1".to_string()),
        ))
        .await
        .unwrap();
    assert!(global_id.starts_with("pinned-session/"));
    assert!(thread_id.starts_with("pinned-session/"));
    {
        let sessions = &state.lock().unwrap().sessions;
        assert_eq!(sessions.len(), 1);
        assert!(sessions.contains_key("pinned-session"));
    }

    // Scope filtering still works inside the shared session.
    let results = store
        .search(query(
            "pinned",
            Some(MemoryScope::Thread("t1".to_string())),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].record.id.as_deref(), Some(thread_id.as_str()));

    let listed = store.list(None, 10).await.unwrap();
    assert_eq!(listed.len(), 2);

    server.abort();
}
