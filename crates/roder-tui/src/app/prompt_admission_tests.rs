use std::sync::Mutex;

use super::super::tests::test_app_with_client;
use super::*;
use tokio::sync::{broadcast, oneshot};

#[derive(Clone)]
struct ControlledClient {
    requests: Arc<Mutex<Vec<JsonRpcRequest>>>,
    replies: Arc<Mutex<VecDeque<oneshot::Receiver<JsonRpcResponse>>>>,
    events: broadcast::Sender<roder_api::events::EventEnvelope>,
    notifications: broadcast::Sender<roder_protocol::JsonRpcNotification>,
}

impl ControlledClient {
    async fn wait_requests(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.requests.lock().unwrap().len() < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("request dispatched");
    }

    fn new(count: usize) -> (Self, VecDeque<oneshot::Sender<JsonRpcResponse>>) {
        let (events, _) = broadcast::channel(16);
        let (notifications, _) = broadcast::channel(16);
        let (senders, receivers): (VecDeque<_>, VecDeque<_>) =
            (0..count).map(|_| oneshot::channel()).unzip();
        (
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                replies: Arc::new(Mutex::new(receivers)),
                events,
                notifications,
            },
            senders,
        )
    }
}

#[async_trait::async_trait]
impl AppClient for ControlledClient {
    type EventReceiver = broadcast::Receiver<roder_api::events::EventEnvelope>;
    type NotificationReceiver = broadcast::Receiver<roder_protocol::JsonRpcNotification>;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        self.requests.lock().unwrap().push(request);
        let receiver = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("planned response");
        receiver.await.unwrap_or(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: None,
            result: None,
            error: None,
        })
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        self.events.subscribe()
    }
    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        self.notifications.subscribe()
    }
}

fn app(client: ControlledClient) -> TuiApp<ControlledClient> {
    test_app_with_client(
        client,
        Arc::new(AppServer::new(Arc::new(
            roder_core::Runtime::fake().unwrap(),
        ))),
    )
}

fn prompt(text: &str) -> PendingPrompt {
    PendingPrompt::with_images(text, text, Vec::new())
}

fn accepted(turn: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: None,
        result: Some(serde_json::json!({"turnId": turn})),
        error: None,
    }
}

fn rejected(message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: None,
        result: None,
        error: Some(roder_protocol::JsonRpcError {
            code: -32000,
            message: message.into(),
            data: None,
        }),
    }
}

async fn settle(app: &mut TuiApp<ControlledClient>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while app
            .pending_turn_requests
            .iter()
            .any(|request| !request.handle.is_finished())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.finish_turn_requests_if_ready().await;
}

#[tokio::test]
async fn pending_start_and_accepted_receipt_hold_the_next_prompt() {
    let (client, mut replies) = ControlledClient::new(2);
    let mut app = app(client.clone());
    app.start_prepared_prompt(prompt("first")).await;
    app.start_prepared_prompt(prompt("second")).await;
    assert_eq!(app.pending_turn_requests.len(), 1);
    assert_eq!(app.queued_prompts.len(), 1);
    replies
        .pop_front()
        .unwrap()
        .send(accepted("turn-first"))
        .unwrap();
    settle(&mut app).await;
    assert!(!app.submit_next_queued_prompt().await);
    assert!(app.active_turn_id.is_none());
    app.prompt_admission
        .note_completed("thread-test", "turn-first");
    assert!(app.submit_next_queued_prompt().await);
    assert!(!app.submit_next_queued_prompt().await);
    replies
        .pop_front()
        .unwrap()
        .send(accepted("turn-second"))
        .unwrap();
    settle(&mut app).await;
    assert_eq!(client.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn queue_waits_for_settings_and_resumes_once_on_close() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client);
    app.show_provider_popup = true;
    app.start_prepared_prompt(prompt("held")).await;
    assert!(app.pending_turn_requests.is_empty());
    assert_eq!(app.queued_prompts.len(), 1);
    assert!(!app.submit_next_queued_prompt().await);
    app.show_provider_popup = false;
    assert!(app.submit_next_queued_prompt().await);
    assert!(!app.submit_next_queued_prompt().await);
    replies.pop_front().unwrap().send(accepted("turn")).unwrap();
    settle(&mut app).await;
}

#[tokio::test]
async fn completed_before_receipt_never_resurrects_or_blocks_the_turn() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client);
    app.start_prepared_prompt(prompt("quick")).await;
    app.prompt_admission
        .note_completed("thread-test", "quick-turn");
    replies
        .pop_front()
        .unwrap()
        .send(accepted("quick-turn"))
        .unwrap();
    settle(&mut app).await;
    assert!(app.active_turn_id.is_none());
    assert!(app.can_dispatch_prompt());
}

#[tokio::test]
async fn rejected_steer_recovers_its_own_payload_without_discarding_later_queue() {
    let (client, mut replies) = ControlledClient::new(3);
    let mut app = app(client.clone());
    app.active_turn_id = Some("live".into());
    let mut first = prompt("A");
    first.images.push(InputImage {
        image_url: "data:image/png;base64,AA==".into(),
    });
    let first_id = first.client_id;
    app.steer_prepared_prompt(first);
    client.wait_requests(1).await;
    app.steer_prepared_prompt(prompt("B"));
    app.queue_prepared_prompt(prompt("C"));
    app.queue_prepared_prompt(prompt("D"));
    // Ensure responses map to submission order, not task scheduling order.
    client.wait_requests(2).await;
    replies
        .pop_front()
        .unwrap()
        .send(rejected("turn is not steerable"))
        .unwrap();
    replies.pop_front().unwrap().send(accepted("live")).unwrap();
    settle(&mut app).await;
    app.active_turn_id = None;
    assert!(!app.submit_next_queued_prompt().await);
    app.run_retry_slash_command("").await;
    assert_eq!(app.pending_turn_requests[0].prompt.client_id, first_id);
    assert_eq!(app.pending_turn_requests[0].prompt.images.len(), 1);
    assert_eq!(
        app.queued_prompts
            .iter()
            .map(|p| p.message.as_str())
            .collect::<Vec<_>>(),
        ["C", "D"]
    );
    replies
        .pop_front()
        .unwrap()
        .send(accepted("retried"))
        .unwrap();
    settle(&mut app).await;
}

#[path = "prompt_recovery_tests.rs"]
mod recovery;

#[tokio::test]
async fn absent_turn_requeues_once_and_never_clears_a_newer_turn() {
    let (client, mut replies) = ControlledClient::new(2);
    let mut app = app(client);
    app.active_turn_id = Some("old".into());
    app.steer_prepared_prompt(prompt("follow-up"));
    app.active_turn_id = Some("newer".into());
    replies
        .pop_front()
        .unwrap()
        .send(rejected("no active turn to steer"))
        .unwrap();
    settle(&mut app).await;
    assert_eq!(app.active_turn_id.as_deref(), Some("newer"));
    assert_eq!(app.queued_prompts.len(), 1);
    assert!(!app.submit_next_queued_prompt().await);
    app.active_turn_id = None;
    assert!(app.submit_next_queued_prompt().await);
    assert_eq!(
        app.thread_message_count, 1,
        "recovery must not render the prompt twice"
    );
    replies
        .pop_front()
        .unwrap()
        .send(rejected("backend unavailable"))
        .unwrap();
    settle(&mut app).await;
    assert!(!app.submit_next_queued_prompt().await);
    assert!(app.prompt_admission.has_recovery("thread-test"));
}

#[tokio::test]
async fn uncertain_response_is_retained_but_never_automatically_resent() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client.clone());
    app.start_prepared_prompt(prompt("possibly accepted")).await;
    app.queue_prepared_prompt(prompt("later"));
    drop(replies.pop_front().unwrap());
    settle(&mut app).await;
    assert!(app.prompt_admission.has_recovery("thread-test"));
    for _ in 0..3 {
        assert!(!app.submit_next_queued_prompt().await);
    }
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    assert_eq!(app.queued_prompts.len(), 1);
}

#[tokio::test]
async fn thread_bound_queue_never_submits_to_another_thread() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client);
    app.queue_prepared_prompt(prompt("original thread"));
    app.thread_id = "another-thread".into();
    assert!(!app.submit_next_queued_prompt().await);
    app.thread_id = "thread-test".into();
    assert!(app.submit_next_queued_prompt().await);
    replies.pop_front().unwrap().send(accepted("turn")).unwrap();
    settle(&mut app).await;
}

#[tokio::test]
async fn lost_request_deadline_retains_input_without_automatic_resend() {
    let (client, _replies) = ControlledClient::new(1);
    let mut app = app(client.clone());
    app.start_prepared_prompt(prompt("unknown acceptance"))
        .await;
    client.wait_requests(1).await;
    app.pending_turn_requests[0].sent_at = Instant::now() - Duration::from_secs(31);
    assert!(app.finish_turn_requests_if_ready().await);
    assert!(app.pending_turn_requests.is_empty());
    assert!(app.prompt_admission.has_recovery("thread-test"));
    assert!(!app.submit_next_queued_prompt().await);
    assert_eq!(client.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn out_of_order_rejections_preserve_submission_order() {
    let (client, mut replies) = ControlledClient::new(2);
    let mut app = app(client.clone());
    app.active_turn_id = Some("turn".into());
    app.steer_prepared_prompt(prompt("A"));
    client.wait_requests(1).await;
    app.steer_prepared_prompt(prompt("B"));
    client.wait_requests(2).await;
    replies
        .pop_back()
        .unwrap()
        .send(rejected("second rejection first"))
        .unwrap();
    while !app.pending_turn_requests[1].handle.is_finished() {
        tokio::task::yield_now().await;
    }
    app.finish_turn_requests_if_ready().await;
    replies
        .pop_front()
        .unwrap()
        .send(rejected("first rejection later"))
        .unwrap();
    settle(&mut app).await;
    assert_eq!(
        app.prompt_admission
            .take_recovery("thread-test")
            .unwrap()
            .message,
        "A"
    );
    assert_eq!(
        app.prompt_admission
            .take_recovery("thread-test")
            .unwrap()
            .message,
        "B"
    );
}

#[tokio::test]
async fn persisted_lifecycle_reconciles_a_lost_completion_without_resending() {
    use roder_api::lifecycle::{
        TurnCleanupState, TurnLifecycleRecord, TurnLifecycleSnapshot, TurnLifecycleState,
    };
    let (client, mut replies) = ControlledClient::new(2);
    let mut app = app(client.clone());
    app.start_prepared_prompt(prompt("accepted")).await;
    replies
        .pop_front()
        .unwrap()
        .send(accepted("completed"))
        .unwrap();
    settle(&mut app).await;
    app.prompt_admission
        .accepted
        .get_mut("thread-test")
        .unwrap()
        .next_probe = Instant::now();
    app.poll_prompt_receipt().await;
    client.wait_requests(2).await;
    let read = roder_protocol::ThreadReadResult {
        thread: None,
        lifecycle: TurnLifecycleSnapshot {
            records: vec![TurnLifecycleRecord::new(
                "thread-test".into(),
                "completed".into(),
                TurnLifecycleState::Completed,
                TurnCleanupState::Completed,
                None,
                time::OffsetDateTime::now_utc(),
            )],
            corrupt_record_count: 0,
        },
    };
    replies
        .pop_front()
        .unwrap()
        .send(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: None,
            result: Some(serde_json::to_value(read).unwrap()),
            error: None,
        })
        .unwrap();
    while !app
        .prompt_admission
        .probe
        .as_ref()
        .unwrap()
        .handle
        .is_finished()
    {
        tokio::task::yield_now().await;
    }
    assert!(app.poll_prompt_receipt().await);
    assert!(app.can_dispatch_prompt());
    assert!(app.active_turn_id.is_none());
    assert_eq!(
        client
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["turn/start", "thread/read"]
    );
}

#[test]
fn resuming_or_switching_threads_preserves_scoped_queue_and_recovery() {
    let (client, _replies) = ControlledClient::new(0);
    let mut app = app(client);
    app.queue_prepared_prompt(prompt("A queued"));
    let mut recovery = prompt("A rejected");
    recovery.thread_id = Some("thread-test".into());
    app.prompt_admission
        .rendered
        .insert(("thread-test".into(), recovery.client_id));
    app.prompt_admission
        .retain_recovery("thread-test".into(), recovery.clone());
    app.apply_thread(super::super::tests::test_thread_with_items(
        "thread-B",
        Vec::new(),
    ));
    assert_eq!(app.queued_prompt_count(), 0);
    app.queue_prepared_prompt(prompt("B queued"));
    assert_eq!(app.queued_prompt_count(), 1);
    assert_eq!(app.queued_prompts.len(), 2);
    app.apply_thread(super::super::tests::test_thread_with_items(
        "thread-test",
        Vec::new(),
    ));
    assert_eq!(app.queued_prompt_count(), 1);
    assert_eq!(
        app.visible_queued_prompts().next().unwrap().1.message,
        "A queued"
    );
    assert!(
        !app.prompt_admission
            .rendered
            .contains(&("thread-test".into(), recovery.client_id))
    );
    assert_eq!(
        app.prompt_admission.take_recovery("thread-test").unwrap(),
        recovery
    );
    assert!(app.edit_latest_queued_prompt());
    assert_eq!(composer_text(&app.composer), "A queued");
    assert_eq!(
        app.queued_prompts.iter().next().unwrap().message,
        "B queued"
    );
}

#[tokio::test]
async fn a_start_error_never_authorizes_absent_steer_recovery() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client);
    app.start_prepared_prompt(prompt("start")).await;
    replies
        .pop_front()
        .unwrap()
        .send(rejected("no active turn to steer"))
        .unwrap();
    settle(&mut app).await;
    assert!(app.queued_prompts.is_empty());
    assert!(app.prompt_admission.has_recovery("thread-test"));
    assert!(!app.submit_next_queued_prompt().await);
}
