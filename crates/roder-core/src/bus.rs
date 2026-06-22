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

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{RoderEvent, TurnStarted};
    use roder_api::inference::RuntimeProfile;

    fn sample_event() -> RoderEvent {
        RoderEvent::TurnStarted(TurnStarted {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            runtime_profile: RuntimeProfile::Interactive,
            timestamp: OffsetDateTime::now_utc(),
        })
    }

    #[tokio::test]
    async fn retains_burst_up_to_capacity_without_lagging() {
        // A slow consumer (the TUI render loop only drains every ~166ms during
        // an active turn) must be able to buffer a large burst of streaming
        // events without the broadcast ring overflowing. This guards the
        // capacity headroom that keeps tool/thinking rows from being dropped.
        let capacity = 16_384;
        let bus = EventBus::new(capacity);
        let mut rx = bus.subscribe();

        for _ in 0..capacity {
            bus.emit(sample_event());
        }

        // Every buffered event is still readable; none were dropped.
        for _ in 0..capacity {
            assert!(rx.try_recv().is_ok());
        }
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn overflow_beyond_capacity_surfaces_lagged() {
        // When the buffer truly overflows the consumer must still observe a
        // `Lagged` signal so the TUI can record the drop and run its stuck-turn
        // recovery instead of hanging.
        let capacity = 16usize;
        let bus = EventBus::new(capacity);
        let mut rx = bus.subscribe();

        for _ in 0..(capacity + 8) {
            bus.emit(sample_event());
        }

        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
        ));
    }
}
