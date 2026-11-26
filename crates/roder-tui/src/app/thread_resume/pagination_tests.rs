use std::sync::Mutex;

use roder_api::events::EventEnvelope;
use roder_protocol::{JsonRpcError, JsonRpcNotification, JsonRpcResponse};
use tokio::sync::{Notify, broadcast};

use super::*;
use crate::app::tests::{test_app_with_client, test_thread_with_items};

#[derive(Clone)]
struct HistoryClient {
    reported_error: Option<JsonRpcError>,
    latest: Thread,
    older: Thread,
    requests: Arc<Mutex<Vec<JsonRpcRequest>>>,
    waiting: Arc<Notify>,
    release: Arc<Notify>,
    events: broadcast::Sender<EventEnvelope>,
    notifications: broadcast::Sender<JsonRpcNotification>,
}

impl HistoryClient {
    fn new(latest: Thread, older: Thread) -> Self {
        Self {
            reported_error: None,
            latest,
            older,
            requests: Arc::new(Mutex::new(Vec::new())),
            waiting: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            events: broadcast::channel(4).0,
            notifications: broadcast::channel(4).0,
        }
    }

    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.method.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl AppClient for HistoryClient {
    type EventReceiver = broadcast::Receiver<EventEnvelope>;
    type NotificationReceiver = broadcast::Receiver<JsonRpcNotification>;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        self.requests.lock().unwrap().push(request.clone());
        let mut response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: None,
        };
        let params = request.params.unwrap();
        response.result = match request.method.as_str() {
            "thread/resume" => {
                assert_eq!(params["limit"], RESUME_VISIBLE_TAIL_ITEMS);
                if let Some(error) = self.reported_error.clone() {
                    response.error = Some(error);
                    return response;
                }
                Some(serde_json::json!({
                    "thread": self.latest,
                    "before": "older-page",
                    "hasMore": true,
                    "lifecycle": {"records": [], "corruptRecordCount": 0}
                }))
            }
            "thread/history/read" if params["before"].is_null() => {
                assert_eq!(params["limit"], RESUME_VISIBLE_TAIL_ITEMS);
                Some(serde_json::json!({
                    "thread": self.latest,
                    "before": "older-page",
                    "hasMore": true,
                }))
            }
            "thread/history/read" => {
                assert_eq!(params["before"], "older-page");
                assert_eq!(params["limit"], RESUME_OLDER_BATCH_ITEMS);
                self.waiting.notify_one();
                self.release.notified().await;
                Some(serde_json::json!({
                    "thread": self.older,
                    "before": null,
                    "hasMore": false,
                }))
            }
            "thread/read" => {
                let mut thread = self.latest.clone();
                if params["includeTurns"] == false {
                    thread.turns = None;
                }
                Some(serde_json::json!({"thread": thread}))
            }
            method => panic!("unexpected method: {method}"),
        };
        response
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        self.events.subscribe()
    }

    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        self.notifications.subscribe()
    }
}

fn user(id: impl ToString, text: impl ToString) -> Item {
    Item::UserMessage {
        id: id.to_string(),
        text: text.to_string(),
        images: Vec::new(),
        status: None,
    }
}

#[tokio::test]
async fn resume_fetches_old_pages_on_scroll_without_overwriting_live_output() {
    let id = "paged-thread";
    let mut latest = test_thread_with_items(
        id,
        (10..30)
            .map(|index| user(index, format!("saved prompt {index}")))
            .collect(),
    );
    latest.status.active_turn_id = Some("live-turn".to_string());
    latest.message_count = Some(100);
    let historical_tool = Item::ToolExecution {
        id: "previous-tool".into(),
        tool_call_id: "previous-tool-call".into(),
        tool_name: "read_file".into(),
        status: ThreadItemStatus::Completed,
        input: Some(serde_json::json!({"path": "old.md"})),
        output: Some("saved tool output".into()),
        error: None,
    };
    let older = test_thread_with_items(
        id,
        vec![
            user(0, "older prompt"),
            historical_tool,
            user(10, "stale overlap"),
        ],
    );
    let client = HistoryClient::new(latest, older);
    let server = Arc::new(AppServer::new(Arc::new(
        roder_core::Runtime::fake().expect("fake runtime"),
    )));
    let mut app = test_app_with_client(client.clone(), server);
    app.load_thread(id.to_string()).await;
    assert_eq!(client.methods(), ["thread/resume"]);
    assert_eq!(app.thread_message_count, 100);
    assert_eq!(app.active_turn_id.as_deref(), Some("live-turn"));
    assert!(app.resume_history.older_items.is_empty());

    app.timeline.render(app.theme, Rect::new(0, 0, 100, 8));
    app.timeline.focus_latest();
    assert!(
        app.timeline
            .handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE,))
    );
    assert_eq!(app.timeline.scroll_offset(), 0);
    app.load_older_resume_history_if_needed();
    app.load_older_resume_history_if_needed();
    tokio::time::timeout(Duration::from_secs(2), client.waiting.notified())
        .await
        .unwrap();
    app.timeline
        .push_assistant_delta_immediate("live output while page is pending", None);
    client.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !app.poll_older_resume_history().await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(client.methods(), ["thread/resume", "thread/history/read"]);
    assert!(
        app.timeline.scroll_offset() > 0,
        "keep the viewport anchored"
    );
    assert!(!app.resume_history.has_older_items());
    assert_eq!(app.resume_history.loaded_items, 22);
    let render = app.timeline.render(app.theme, Rect::new(0, 0, 100, 250));
    let text = render.text.to_string();
    assert!(text.contains("older prompt"));
    assert!(text.contains("old.md"));
    assert!(text.contains("live output while page is pending"));
    assert_eq!(text.matches("saved prompt 10").count(), 1);
    assert!(!text.contains("stale overlap"));
}

#[tokio::test]
async fn resume_falls_back_only_when_pagination_is_unavailable() {
    let thread = test_thread_with_items("thread", vec![user(1, "saved prompt")]);
    for (code, message, fallback) in [
        (-32601, "method not found", true),
        (
            -32000,
            "thread store does not support bounded history pagination",
            true,
        ),
        (-32000, "thread/resume is unsupported by this backend", true),
        (-32000, "history index read failed", false),
        (-32012, "forbidden: inactive_tenant", false),
    ] {
        let mut client = HistoryClient::new(thread.clone(), thread.clone());
        client.reported_error = Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        });
        let result = read_thread(&client, &thread.id).await;
        assert_eq!(result.is_ok(), fallback);
        assert_eq!(
            client.methods(),
            if fallback {
                vec!["thread/resume", "thread/read"]
            } else {
                vec!["thread/resume"]
            }
        );
    }
}

#[tokio::test]
async fn read_only_session_browses_history_without_resume_side_effects() {
    let thread = test_thread_with_items("thread", vec![user(1, "saved prompt")]);
    let mut client = HistoryClient::new(thread.clone(), thread.clone());
    client.reported_error = Some(JsonRpcError {
        code: -32012,
        message: "forbidden: missing_scope".to_string(),
        data: None,
    });

    let read = read_thread(&client, &thread.id).await.unwrap();
    assert_eq!(read.thread.unwrap().turns.unwrap()[0].items.len(), 1);
    assert_eq!(read.before.as_deref(), Some("older-page"));
    assert_eq!(
        client.methods(),
        ["thread/resume", "thread/history/read", "thread/read"]
    );
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests[2].params.as_ref().unwrap()["includeTurns"], false);
}
