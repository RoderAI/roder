//! `EventSink` adapter that forwards filtered canonical envelopes to the
//! child as `events/handle` notifications.

use std::sync::Arc;
use std::time::Duration;

use roder_api::events::EventEnvelope;
use roder_api::extension::{EventSink, EventSinkId};
use roder_api::process_extension::{METHOD_EVENTS_HANDLE, ProcessEventsHandleNotification};

use crate::process::ProcessHost;

/// Upper bound on time spent handing one event to the child. The runtime
/// dispatcher additionally isolates sinks, so this only protects the
/// notification write itself.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(2);

pub struct ProcessEventSink {
    host: Arc<ProcessHost>,
    sink_id: String,
}

impl ProcessEventSink {
    pub fn new(host: Arc<ProcessHost>, sink_id: String) -> Self {
        Self { host, sink_id }
    }
}

#[async_trait::async_trait]
impl EventSink for ProcessEventSink {
    fn id(&self) -> EventSinkId {
        self.sink_id.clone()
    }

    async fn handle_event(&self, envelope: &EventEnvelope) -> anyhow::Result<()> {
        if !self
            .host
            .loaded()
            .config
            .event_filter
            .matches(&envelope.kind)
        {
            return Ok(());
        }
        let params = serde_json::to_value(ProcessEventsHandleNotification {
            envelope: envelope.clone(),
        })?;
        tokio::time::timeout(FORWARD_TIMEOUT, self.host.notify(METHOD_EVENTS_HANDLE, params))
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "event sink {} timed out forwarding {}",
                    self.sink_id,
                    envelope.kind
                )
            })?
    }
}
