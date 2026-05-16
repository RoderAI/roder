use std::sync::atomic::Ordering;

use roder_api::events::{EventEnvelope, RoderEvent, TranscriptOpenFileRequested};
use roder_protocol::{JsonRpcError, TranscriptOpenFileParams, TranscriptOpenFileResult};
use time::OffsetDateTime;

use super::AppServer;

impl AppServer {
    pub(crate) async fn handle_transcript_open_file(
        &self,
        params: TranscriptOpenFileParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let event = RoderEvent::TranscriptOpenFileRequested(TranscriptOpenFileRequested {
            thread_id: params.thread_id,
            path: params.path,
            line: params.line,
            timestamp: OffsetDateTime::now_utc(),
        });
        let envelope = EventEnvelope {
            event_id: uuid::Uuid::new_v4().to_string(),
            seq: self.event_seq.fetch_add(1, Ordering::SeqCst) + 1,
            timestamp: OffsetDateTime::now_utc(),
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };
        let _ = self.events.send(envelope);
        Ok(serde_json::to_value(TranscriptOpenFileResult { requested: true }).unwrap())
    }
}
