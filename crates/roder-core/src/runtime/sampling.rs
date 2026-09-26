use super::*;
use roder_api::provider_error::ProviderFailure;
use tokio::sync::watch;

#[derive(Debug)]
pub(super) struct SamplingPreempted;
impl std::fmt::Display for SamplingPreempted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sampling preempted by new input")
    }
}
impl std::error::Error for SamplingPreempted {}

pub(super) async fn preemptible<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
    steering: &mut watch::Receiver<u64>,
) -> anyhow::Result<T> {
    tokio::select! {
        biased;
        _ = wait_for_steer(steering) => Err(SamplingPreempted.into()),
        result = future => result,
    }
}

pub(super) struct SamplingRetry<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub config: &'a RuntimeReliabilityConfig,
    pub error: &'a anyhow::Error,
}

pub(super) fn completed_call_replayed(
    transcript: &[TranscriptItem],
    call: &ToolCallCompleted,
) -> anyhow::Result<bool> {
    if !transcript
        .iter()
        .any(|item| matches!(item, TranscriptItem::ToolResult(result) if result.id == call.id))
    {
        return Ok(false);
    }
    if let Some(previous) = transcript.iter().find_map(|item| match item {
        TranscriptItem::ToolCall(previous) if previous.id == call.id => Some(previous),
        _ => None,
    }) {
        let same_arguments = previous.arguments == call.arguments
            || serde_json::from_str::<serde_json::Value>(&previous.arguments)
                .ok()
                .zip(serde_json::from_str::<serde_json::Value>(&call.arguments).ok())
                .is_some_and(|(previous, current)| previous == current);
        if previous.name != call.name || !same_arguments {
            return Err(roder_api::provider_error::ProviderFailure::new(
                roder_api::provider_error::ProviderFailureKind::Protocol,
                format!(
                    "provider reused completed tool call id {} with different arguments",
                    call.id
                ),
            )
            .into());
        }
    }
    Ok(true)
}

impl Runtime {
    pub(super) async fn finish_sampling_cleanup(&self, turn_id: &TurnId) -> anyhow::Result<()> {
        let (state, ownership) = self.await_provider_turn_cleanup(turn_id).await;
        if state != TurnCleanupState::Completed
            && ownership != TurnCleanupOwnership::RuntimeTaskOnly
        {
            anyhow::bail!("provider sampling cleanup was not confirmed: {state:?}");
        }
        Ok(())
    }

    pub(super) async fn drain_turn_steers(
        &self,
        turn_id: &TurnId,
        steering: &mut watch::Receiver<u64>,
    ) -> Vec<QueuedTurnSteer> {
        let Some(active) = self.active_turns.read().await.get(turn_id).cloned() else {
            return Vec::new();
        };
        let mut steers = active.steers.lock().await;
        // Enqueue sends the watch update under this same lock. Marking it seen
        // together with draining cannot lose input or leave a stale notification.
        steering.borrow_and_update();
        std::mem::take(&mut *steers)
    }

    pub(super) async fn retry_sampling_failure(
        &self,
        retry: SamplingRetry<'_>,
        attempts: &mut u32,
        steering: &mut watch::Receiver<u64>,
    ) -> bool {
        let failure = retry.error.downcast_ref::<ProviderFailure>();
        if let Some(failure) = failure {
            let _ = self
                .persist_turn_item(
                    &retry.thread_id.to_string(),
                    &retry.turn_id.to_string(),
                    &TranscriptItem::ProviderMetadata(failure.metadata()),
                )
                .await;
        }
        let cause = if let Some(failure) = failure {
            if !failure.kind.is_retryable() || failure.retry_budget_exhausted {
                return false;
            }
            failure.kind.retry_cause()
        } else if let Some(cause) = provider_stream_retry_cause(&retry.error.to_string()) {
            cause
        } else {
            return false;
        };
        let attempt = attempts.saturating_add(1);
        let policy: ReliabilityRequestPolicy = retry.config.clone().into();
        if attempt >= policy.provider_retry_max_attempts.max(1) {
            return false;
        }
        *attempts = attempt;
        let local = tokio::time::Instant::now()
            + std::time::Duration::from_millis(provider_retry_delay_ms(&policy, attempt));
        let deadline = failure
            .and_then(|failure| failure.retry_not_before)
            .map_or(local, |server| server.max(local));
        let delay_ms = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.emit(RoderEvent::ReliabilityRetryRecorded(
            ReliabilityRetryRecorded {
                context: ReliabilityContext {
                    thread_id: retry.thread_id.to_string(),
                    turn_id: retry.turn_id.to_string(),
                    provider: Some(retry.provider.to_string()),
                    model: Some(retry.model.to_string()),
                    ..Default::default()
                },
                error_class: ReliabilityErrorClass::ProviderError,
                decision: ReliabilityRetryDecision::Retry,
                attempt,
                max_attempts: policy.provider_retry_max_attempts,
                delay_ms: Some(delay_ms),
                details: ReliabilityDetails::redacted(format!("{cause}: {}", retry.error)),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        let _ = preemptible(
            async {
                tokio::time::sleep_until(deadline).await;
                Ok(())
            },
            steering,
        )
        .await;
        true
    }
}

/// Closing the steering channel accompanies turn cancellation. It is not new
/// input; leave cancellation to the turn's Abortable owner instead of spinning.
pub(crate) async fn wait_for_steer(steering: &mut watch::Receiver<u64>) {
    if steering.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn closed_steering_channel_does_not_preempt_or_spin() {
        let (sender, mut receiver) = watch::channel(0);
        drop(sender);
        assert_eq!(
            preemptible(async { Ok(7) }, &mut receiver).await.unwrap(),
            7
        );
    }
}
