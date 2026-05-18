use std::sync::Arc;

use roder_api::events::{
    RoderEvent, SubagentTraceCompleted, SubagentTraceCreated, SubagentTraceDeltaEvent,
    SubagentTraceFailed, SubagentTraceStatusChanged,
};
use roder_api::session::SessionStore;
use roder_api::trace::{
    ParentTurnRef, SubagentTraceDelta, SubagentTraceId, SubagentTraceSink, SubagentTraceStatus,
    SubagentTraceSummary,
};
use time::OffsetDateTime;

use crate::bus::EventBus;

#[derive(Clone)]
pub(crate) struct RuntimeSubagentTraceSink {
    bus: EventBus,
    session_store: Option<Arc<dyn SessionStore>>,
}

impl RuntimeSubagentTraceSink {
    pub(crate) fn new(bus: EventBus, session_store: Option<Arc<dyn SessionStore>>) -> Self {
        Self { bus, session_store }
    }

    async fn emit(&self, event: RoderEvent) {
        let envelope = self.bus.emit(event);
        if let (Some(store), Some(thread_id)) = (&self.session_store, envelope.thread_id.as_ref()) {
            let _ = store.append_event(thread_id, &envelope).await;
        }
    }
}

#[async_trait::async_trait]
impl SubagentTraceSink for RuntimeSubagentTraceSink {
    async fn trace_created(&self, summary: SubagentTraceSummary) {
        self.emit(RoderEvent::SubagentTraceCreated(SubagentTraceCreated {
            summary,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    async fn trace_delta(&self, delta: SubagentTraceDelta) {
        self.emit(RoderEvent::SubagentTraceDelta(SubagentTraceDeltaEvent {
            delta,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    async fn trace_status_changed(
        &self,
        trace_id: SubagentTraceId,
        parent: ParentTurnRef,
        status: SubagentTraceStatus,
        detail: Option<String>,
    ) {
        self.emit(RoderEvent::SubagentTraceStatusChanged(
            SubagentTraceStatusChanged {
                trace_id,
                parent,
                status,
                detail,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    async fn trace_completed(&self, summary: SubagentTraceSummary) {
        self.emit(RoderEvent::SubagentTraceCompleted(SubagentTraceCompleted {
            summary,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    async fn trace_failed(&self, summary: SubagentTraceSummary, error: String) {
        self.emit(RoderEvent::SubagentTraceFailed(SubagentTraceFailed {
            summary,
            error,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::trace::{SubagentDestination, SubagentDestinationKind};

    #[tokio::test]
    async fn subagent_trace_sink_emits_parent_turn_envelope() {
        let bus = EventBus::new(16);
        let sink = RuntimeSubagentTraceSink::new(bus.clone(), None);
        let mut events = bus.subscribe();

        sink.trace_created(SubagentTraceSummary {
            trace_id: "trace-1".to_string(),
            parent: ParentTurnRef {
                thread_id: "parent-thread".to_string(),
                turn_id: "parent-turn".to_string(),
            },
            child_thread_id: "child-thread".to_string(),
            child_turn_id: "child-turn".to_string(),
            title: "Inspect".to_string(),
            role: "explore".to_string(),
            model: Some("mock".to_string()),
            status: SubagentTraceStatus::Queued,
            elapsed_ms: 0,
            usage: None,
            destination: Some(SubagentDestination {
                kind: SubagentDestinationKind::InProcess,
                label: "in-process".to_string(),
                path: None,
                provider_id: None,
                destination_id: None,
            }),
            latest_activity: Some("queued".to_string()),
            error_summary: None,
        })
        .await;

        let envelope = events.recv().await.unwrap();
        assert_eq!(envelope.kind, "turn/subagentTraceCreated");
        assert_eq!(envelope.thread_id.as_deref(), Some("parent-thread"));
        assert_eq!(envelope.turn_id.as_deref(), Some("parent-turn"));
    }
}
