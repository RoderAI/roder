//! Bounded, non-blocking dispatch of persisted runtime events to registered
//! `EventSink`s (roadmap phase 64, Task 3).
//!
//! Each sink gets its own bounded queue and worker task so a slow or
//! crashed sink (e.g. a wedged process extension) never blocks `emit` or
//! turn progress. Failures and timeouts surface as redacted
//! `extension.event_sink_failed` events, which are themselves never
//! re-dispatched to sinks — a broken sink cannot create an event loop.

use std::sync::Arc;
use std::time::Duration;

use roder_api::events::{EventEnvelope, EventSinkFailed, RoderEvent};
use roder_api::extension::EventSink;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::bus::EventBus;

/// Per-sink queue depth; overflowing events are dropped with a failure event.
const QUEUE_DEPTH: usize = 256;
/// Upper bound on one sink handling one event.
const HANDLE_TIMEOUT: Duration = Duration::from_secs(2);
/// Failure messages are truncated to keep redacted diagnostics bounded.
const MAX_FAILURE_MESSAGE: usize = 300;

pub(crate) struct EventSinkDispatcher {
    workers: Vec<Worker>,
}

struct Worker {
    sink_id: String,
    tx: mpsc::Sender<EventEnvelope>,
}

impl EventSinkDispatcher {
    /// Spawns one worker per sink. Must be called from a tokio context.
    pub(crate) fn start(sinks: &[Arc<dyn EventSink>], bus: EventBus) -> Self {
        let workers = sinks
            .iter()
            .map(|sink| {
                let (tx, mut rx) = mpsc::channel::<EventEnvelope>(QUEUE_DEPTH);
                let sink = sink.clone();
                let sink_id = sink.id();
                let worker_bus = bus.clone();
                let worker_sink_id = sink_id.clone();
                tokio::spawn(async move {
                    while let Some(envelope) = rx.recv().await {
                        let outcome =
                            tokio::time::timeout(HANDLE_TIMEOUT, sink.handle_event(&envelope))
                                .await;
                        let failure = match outcome {
                            Ok(Ok(())) => None,
                            Ok(Err(error)) => Some(truncate_message(&error.to_string())),
                            Err(_) => {
                                Some(format!("timed out after {}ms", HANDLE_TIMEOUT.as_millis()))
                            }
                        };
                        if let Some(message) = failure {
                            worker_bus.emit(RoderEvent::EventSinkFailed(EventSinkFailed {
                                sink_id: worker_sink_id.clone(),
                                event_kind: envelope.kind.clone(),
                                message,
                                timestamp: OffsetDateTime::now_utc(),
                            }));
                        }
                    }
                });
                Worker { sink_id, tx }
            })
            .collect();
        Self { workers }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    /// Queues `envelope` for every sink without blocking. Queue overflow
    /// drops the event for that sink and records a failure event.
    pub(crate) fn dispatch(&self, envelope: &EventEnvelope, bus: &EventBus) {
        if envelope.kind == "extension.event_sink_failed" {
            return;
        }
        for worker in &self.workers {
            if let Err(mpsc::error::TrySendError::Full(_)) = worker.tx.try_send(envelope.clone()) {
                bus.emit(RoderEvent::EventSinkFailed(EventSinkFailed {
                    sink_id: worker.sink_id.clone(),
                    event_kind: envelope.kind.clone(),
                    message: format!("queue full ({QUEUE_DEPTH}); event dropped"),
                    timestamp: OffsetDateTime::now_utc(),
                }));
            }
        }
    }
}

fn truncate_message(message: &str) -> String {
    if message.len() <= MAX_FAILURE_MESSAGE {
        return message.to_string();
    }
    let mut end = MAX_FAILURE_MESSAGE;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}
