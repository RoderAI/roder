use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
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
    publication: Arc<Mutex<Publication>>,
}

struct Publication {
    next: u64,
    ready: BTreeMap<u64, Option<EventEnvelope>>,
}

/// A cancelled persistence future must release its sequence slot too.
pub(crate) struct PreparedEvent {
    bus: EventBus,
    envelope: EventEnvelope,
    finished: bool,
}

impl PreparedEvent {
    pub(crate) fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }

    pub(crate) fn publish(mut self) {
        self.finished = true;
        self.bus
            .finish(self.envelope.seq, Some(self.envelope.clone()));
    }
}

impl Drop for PreparedEvent {
    fn drop(&mut self) {
        if !self.finished {
            self.bus.finish(self.envelope.seq, None);
        }
    }
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            next_seq: Arc::new(AtomicU64::new(1)),
            publication: Arc::new(Mutex::new(Publication {
                next: 1,
                ready: BTreeMap::new(),
            })),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }

    pub fn emit(&self, event: RoderEvent) -> EventEnvelope {
        let prepared = self.prepare(event);
        let envelope = prepared.envelope().clone();
        prepared.publish();
        envelope
    }

    pub(crate) fn prepare(&self, event: RoderEvent) -> PreparedEvent {
        PreparedEvent {
            bus: self.clone(),
            finished: false,
            envelope: EventEnvelope {
                event_id: uuid::Uuid::new_v4().to_string(),
                seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
                timestamp: OffsetDateTime::now_utc(),
                source: event.source(),
                kind: event.kind().to_string(),
                thread_id: event.thread_id().cloned(),
                turn_id: event.turn_id().cloned(),
                event,
            },
        }
    }

    fn finish(&self, sequence: u64, envelope: Option<EventEnvelope>) {
        let mut publication = self
            .publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        publication.ready.insert(sequence, envelope);
        loop {
            let next = publication.next;
            let Some(envelope) = publication.ready.remove(&next) else {
                break;
            };
            publication.next += 1;
            if let Some(envelope) = envelope {
                let _ = self.sender.send(envelope);
            }
        }
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

    #[test]
    fn prepared_events_preserve_order_and_abandoned_slots_do_not_block_subscribers() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe();
        let first = bus.prepare(sample_event());
        let second = bus.emit(sample_event());
        assert!(receiver.try_recv().is_err());
        let first_sequence = first.envelope().seq;
        first.publish();
        assert_eq!(receiver.try_recv().unwrap().seq, first_sequence);
        assert_eq!(receiver.try_recv().unwrap().seq, second.seq);
        let abandoned = bus.prepare(sample_event());
        let following = bus.emit(sample_event());
        assert!(receiver.try_recv().is_err());
        drop(abandoned);
        assert_eq!(receiver.try_recv().unwrap().seq, following.seq);
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
