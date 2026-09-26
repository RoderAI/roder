use futures::{Stream, StreamExt, stream};
use roder_api::inference::{CompletionMetadata, InferenceEvent, InferenceEventStream};

/// Every synthetic tool response must obey the same terminal contract as a
/// provider response; tool calls alone cannot make EOF a successful outcome.
pub(super) fn completed(
    events: impl Stream<Item = anyhow::Result<InferenceEvent>> + Send + 'static,
) -> InferenceEventStream {
    Box::pin(events.chain(stream::iter([Ok(InferenceEvent::Completed(
        CompletionMetadata {
            stop_reason: Some("tool_calls".into()),
            provider_response_id: None,
        },
    ))])))
}
