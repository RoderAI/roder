use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use roder_api::events::{EventEnvelope, EventSource, RoderEvent, ThreadId, TurnId};
use time::OffsetDateTime;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub kinds: Vec<String>,
    pub sources: Vec<EventSource>,
}

impl EventFilter {
    pub fn matches(&self, envelope: &EventEnvelope) -> bool {
        if let Some(thread_id) = &self.thread_id
            && envelope.thread_id.as_ref() != Some(thread_id)
        {
            return false;
        }
        if let Some(turn_id) = &self.turn_id
            && envelope.turn_id.as_ref() != Some(turn_id)
        {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|kind| kind == &envelope.kind) {
            return false;
        }
        if !self.sources.is_empty() && !self.sources.iter().any(|source| source == &envelope.source)
        {
            return false;
        }
        true
    }
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<EventEnvelope>,
    next_seq: Arc<AtomicU64>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            next_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }

    pub fn emit(&self, event: RoderEvent) -> EventEnvelope {
        let envelope = EventEnvelope {
            event_id: uuid::Uuid::new_v4().to_string(),
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            timestamp: OffsetDateTime::now_utc(),
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };
        let _ = self.sender.send(envelope.clone());
        envelope
    }
}
