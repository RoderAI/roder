use roder_api::events::{EventEnvelope, RoderEvent, EventId};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<EventEnvelope>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }

    pub fn emit(&self, event: RoderEvent) {
        let envelope = EventEnvelope {
            event_id: uuid::Uuid::new_v4().to_string(),
            event,
        };
        // We ignore the error here which occurs if there are no subscribers.
        let _ = self.sender.send(envelope);
    }
}
