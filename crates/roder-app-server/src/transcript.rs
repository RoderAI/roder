use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use roder_api::events::EventEnvelope;
use roder_api_transcript::{ApiTranscriptRecord, write_jsonl_record};
use roder_protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use tokio::sync::broadcast;

use crate::client::{AppClient, AppEventReceiver, AppNotificationReceiver};

#[derive(Clone, Default)]
pub struct TranscriptRecorder {
    state: Arc<Mutex<RecorderState>>,
}

#[derive(Default)]
struct RecorderState {
    next_seq: u64,
    started_at: Option<Instant>,
    records: Vec<ApiTranscriptRecord>,
    jsonl: Vec<u8>,
}

impl TranscriptRecorder {
    pub fn push(&self, record: ApiTranscriptRecord) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("transcript recorder mutex poisoned");
        write_jsonl_record(&mut state.jsonl, &record)?;
        state.records.push(record);
        Ok(())
    }

    pub fn next_seq_at_ms(&self) -> (u64, u64) {
        let mut state = self
            .state
            .lock()
            .expect("transcript recorder mutex poisoned");
        let started_at = *state.started_at.get_or_insert_with(Instant::now);
        state.next_seq = state.next_seq.saturating_add(1);
        (
            state.next_seq,
            Instant::now()
                .saturating_duration_since(started_at)
                .as_millis() as u64,
        )
    }

    pub fn records(&self) -> Vec<ApiTranscriptRecord> {
        self.state
            .lock()
            .expect("transcript recorder mutex poisoned")
            .records
            .clone()
    }

    pub fn jsonl(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("transcript recorder mutex poisoned")
            .jsonl
            .clone()
    }
}

#[derive(Clone)]
pub struct RecordingAppClient<C> {
    inner: C,
    recorder: TranscriptRecorder,
    client_id: String,
}

impl<C> RecordingAppClient<C> {
    pub fn new(inner: C, recorder: TranscriptRecorder, client_id: impl Into<String>) -> Self {
        Self {
            inner,
            recorder,
            client_id: client_id.into(),
        }
    }

    pub fn recorder(&self) -> TranscriptRecorder {
        self.recorder.clone()
    }
}

#[async_trait]
impl<C> AppClient for RecordingAppClient<C>
where
    C: AppClient,
{
    type EventReceiver = RecordingEventReceiver<C::EventReceiver>;
    type NotificationReceiver = RecordingNotificationReceiver<C::NotificationReceiver>;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let (request_seq, request_at_ms) = self.recorder.next_seq_at_ms();
        let request_value = serde_json::to_value(&request).unwrap_or_else(|err| {
            serde_json::json!({
                "serializationError": err.to_string()
            })
        });
        let _ = self.recorder.push(ApiTranscriptRecord::ApiRequest {
            seq: request_seq,
            at_ms: request_at_ms,
            client: self.client_id.clone(),
            request: request_value,
        });

        let response = self.inner.send_request(request).await;
        let response_value = serde_json::to_value(&response).unwrap_or_else(|err| {
            serde_json::json!({
                "serializationError": err.to_string()
            })
        });
        let (response_seq, response_at_ms) = self.recorder.next_seq_at_ms();
        let _ = self.recorder.push(ApiTranscriptRecord::ApiResponse {
            seq: response_seq,
            at_ms: response_at_ms,
            request_seq,
            response: response_value,
        });
        response
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        RecordingEventReceiver {
            inner: self.inner.subscribe_events(),
            recorder: self.recorder.clone(),
            stream: "runtime.events".to_string(),
        }
    }

    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        RecordingNotificationReceiver {
            inner: self.inner.subscribe_notifications(),
            recorder: self.recorder.clone(),
            stream: "api.notifications".to_string(),
        }
    }
}

pub struct RecordingEventReceiver<R> {
    inner: R,
    recorder: TranscriptRecorder,
    stream: String,
}

#[async_trait]
impl<R> AppEventReceiver for RecordingEventReceiver<R>
where
    R: AppEventReceiver,
{
    async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError> {
        match self.inner.recv().await {
            Ok(envelope) => {
                let value = serde_json::to_value(&envelope).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serializationError": err.to_string()
                    })
                });
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::RuntimeEvent {
                    seq,
                    at_ms,
                    envelope: value,
                });
                Ok(envelope)
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::BroadcastLag {
                    seq,
                    at_ms,
                    stream: self.stream.clone(),
                    skipped,
                });
                Err(broadcast::error::RecvError::Lagged(skipped))
            }
            Err(err) => Err(err),
        }
    }

    fn try_recv(&mut self) -> Result<EventEnvelope, broadcast::error::TryRecvError> {
        match self.inner.try_recv() {
            Ok(envelope) => {
                let value = serde_json::to_value(&envelope).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serializationError": err.to_string()
                    })
                });
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::RuntimeEvent {
                    seq,
                    at_ms,
                    envelope: value,
                });
                Ok(envelope)
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::BroadcastLag {
                    seq,
                    at_ms,
                    stream: self.stream.clone(),
                    skipped,
                });
                Err(broadcast::error::TryRecvError::Lagged(skipped))
            }
            Err(err) => Err(err),
        }
    }
}

pub struct RecordingNotificationReceiver<R> {
    inner: R,
    recorder: TranscriptRecorder,
    stream: String,
}

#[async_trait]
impl<R> AppNotificationReceiver for RecordingNotificationReceiver<R>
where
    R: AppNotificationReceiver,
{
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError> {
        match self.inner.recv().await {
            Ok(notification) => {
                let value = serde_json::to_value(&notification).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serializationError": err.to_string()
                    })
                });
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::ApiNotification {
                    seq,
                    at_ms,
                    notification: value,
                });
                Ok(notification)
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::BroadcastLag {
                    seq,
                    at_ms,
                    stream: self.stream.clone(),
                    skipped,
                });
                Err(broadcast::error::RecvError::Lagged(skipped))
            }
            Err(err) => Err(err),
        }
    }

    fn try_recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::TryRecvError> {
        match self.inner.try_recv() {
            Ok(notification) => {
                let value = serde_json::to_value(&notification).unwrap_or_else(|err| {
                    serde_json::json!({
                        "serializationError": err.to_string()
                    })
                });
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::ApiNotification {
                    seq,
                    at_ms,
                    notification: value,
                });
                Ok(notification)
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                let (seq, at_ms) = self.recorder.next_seq_at_ms();
                let _ = self.recorder.push(ApiTranscriptRecord::BroadcastLag {
                    seq,
                    at_ms,
                    stream: self.stream.clone(),
                    skipped,
                });
                Err(broadcast::error::TryRecvError::Lagged(skipped))
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{EventEnvelope, EventSource, RoderEvent, RuntimeStarted};
    use roder_protocol::{JsonRpcError, JsonRpcResponse};
    use serde_json::json;
    use time::OffsetDateTime;

    use super::*;

    #[derive(Clone)]
    struct FakeClient {
        events: broadcast::Sender<EventEnvelope>,
        notifications: broadcast::Sender<JsonRpcNotification>,
    }

    #[async_trait]
    impl AppClient for FakeClient {
        type EventReceiver = broadcast::Receiver<EventEnvelope>;
        type NotificationReceiver = broadcast::Receiver<JsonRpcNotification>;

        async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(json!({"ok": true})),
                error: None,
            }
        }

        fn subscribe_events(&self) -> Self::EventReceiver {
            self.events.subscribe()
        }

        fn subscribe_notifications(&self) -> Self::NotificationReceiver {
            self.notifications.subscribe()
        }
    }

    #[tokio::test]
    async fn recording_client_writes_request_response_event_and_notification_order() {
        let (event_tx, _) = broadcast::channel(8);
        let (notification_tx, _) = broadcast::channel(8);
        let recorder = TranscriptRecorder::default();
        let client = RecordingAppClient::new(
            FakeClient {
                events: event_tx.clone(),
                notifications: notification_tx.clone(),
            },
            recorder.clone(),
            "tui",
        );
        let mut events = client.subscribe_events();
        let mut notifications = client.subscribe_notifications();

        let response = client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(1)),
                method: "session/get".to_string(),
                params: None,
            })
            .await;
        assert!(response.error.is_none());

        event_tx.send(runtime_started()).unwrap();
        events.recv().await.unwrap();
        notification_tx
            .send(JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "session/changed".to_string(),
                params: json!({"threadId": "thread-a"}),
            })
            .unwrap();
        notifications.recv().await.unwrap();

        let records = recorder.records();
        let kinds = records
            .iter()
            .map(ApiTranscriptRecord::transcript_kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                roder_api_transcript::ApiTranscriptKind::ApiRequest,
                roder_api_transcript::ApiTranscriptKind::ApiResponse,
                roder_api_transcript::ApiTranscriptKind::RuntimeEvent,
                roder_api_transcript::ApiTranscriptKind::ApiNotification,
            ]
        );
        assert!(
            String::from_utf8(recorder.jsonl())
                .unwrap()
                .contains("session/get")
        );
    }

    #[tokio::test]
    async fn recording_receiver_records_broadcast_lag() {
        let receiver = LaggedEventReceiver;
        let recorder = TranscriptRecorder::default();
        let mut receiver = RecordingEventReceiver {
            inner: receiver,
            recorder: recorder.clone(),
            stream: "runtime.events".to_string(),
        };

        let err = receiver.recv().await.unwrap_err();

        assert!(matches!(err, broadcast::error::RecvError::Lagged(4)));
        assert!(matches!(
            recorder.records().as_slice(),
            [ApiTranscriptRecord::BroadcastLag {
                stream,
                skipped: 4,
                ..
            }] if stream == "runtime.events"
        ));
    }

    struct LaggedEventReceiver;

    #[async_trait]
    impl AppEventReceiver for LaggedEventReceiver {
        async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError> {
            Err(broadcast::error::RecvError::Lagged(4))
        }

        fn try_recv(&mut self) -> Result<EventEnvelope, broadcast::error::TryRecvError> {
            Err(broadcast::error::TryRecvError::Lagged(4))
        }
    }

    fn runtime_started() -> EventEnvelope {
        EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: "runtime.started".to_string(),
            thread_id: None,
            turn_id: None,
            event: RoderEvent::RuntimeStarted(RuntimeStarted {
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        }
    }

    #[allow(dead_code)]
    fn _error_response() -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: "internal".to_string(),
                data: None,
            }),
        }
    }
}
