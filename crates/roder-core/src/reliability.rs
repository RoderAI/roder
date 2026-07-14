use roder_api::reliability::{
    ReliabilityErrorClass, ReliabilityLimitDecision, ReliabilityLimitKind, ReliabilityRequestPolicy,
};
use roder_api::transcript::ToolResultRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeReliabilityConfig {
    pub max_consecutive_tool_failures: u32,
    pub max_tool_failures_per_turn: u32,
    pub max_model_calls_per_turn: u32,
    /// When true, hitting `max_consecutive_tool_failures` requests a turn
    /// continuation (the runtime resets the consecutive counter and nudges the
    /// model to keep going) instead of stopping the turn. Loop persistence for
    /// eval-style runs where a couple of failed tool calls should not end an
    /// otherwise-productive turn. Progress stays bounded by
    /// `max_tool_failures_per_turn` and `max_model_calls_per_turn`, which still
    /// stop the turn. Defaults to `false` so interactive/non-interactive UX is
    /// unchanged; the eval profile enables it.
    pub continue_on_failure_limit: bool,
    /// Number of times a non-interactive/eval turn may be nudged to keep working
    /// when the model returns a final message with no tool calls. `0` disables
    /// the nudge (default) so interactive completions end promptly.
    pub empty_tool_call_nudges: u32,
    pub provider_retry_max_attempts: u32,
    pub provider_retry_initial_backoff_ms: u64,
    pub provider_retry_backoff_factor: u32,
    pub provider_retry_status_codes: Vec<u16>,
    pub retry_empty_provider_body: bool,
}

impl Default for RuntimeReliabilityConfig {
    fn default() -> Self {
        Self {
            max_consecutive_tool_failures: 5,
            max_tool_failures_per_turn: 128,
            max_model_calls_per_turn: 512,
            continue_on_failure_limit: false,
            empty_tool_call_nudges: 0,
            provider_retry_max_attempts: 3,
            provider_retry_initial_backoff_ms: 1_000,
            provider_retry_backoff_factor: 2,
            provider_retry_status_codes: vec![429, 500, 502, 503, 504],
            retry_empty_provider_body: true,
        }
    }
}

impl From<RuntimeReliabilityConfig> for ReliabilityRequestPolicy {
    fn from(config: RuntimeReliabilityConfig) -> Self {
        Self {
            provider_retry_max_attempts: config.provider_retry_max_attempts,
            provider_retry_initial_backoff_ms: config.provider_retry_initial_backoff_ms,
            provider_retry_backoff_factor: config.provider_retry_backoff_factor,
            retry_empty_provider_body: config.retry_empty_provider_body,
            provider_retry_status_codes: config.provider_retry_status_codes,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TurnReliabilityState {
    model_calls: u32,
    consecutive_tool_failures: u32,
    tool_failures: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReliabilityLimitHit {
    pub error_class: ReliabilityErrorClass,
    pub limit_kind: ReliabilityLimitKind,
    pub decision: ReliabilityLimitDecision,
    pub current: u32,
    pub limit: u32,
    pub message: String,
}

pub(crate) fn provider_stream_retry_cause(message: &str) -> Option<&'static str> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("error decoding response body") {
        return Some("stream_decode_error");
    }
    if lower.contains("stream closed before response.completed") {
        return Some("stream_closed_before_completed");
    }
    if lower.contains("stream closed before message_stop") {
        return Some("stream_closed_before_message_stop");
    }
    None
}

impl TurnReliabilityState {
    pub(crate) fn record_model_call(
        &mut self,
        cfg: &RuntimeReliabilityConfig,
        interactive: bool,
    ) -> Option<ReliabilityLimitHit> {
        self.model_calls = self.model_calls.saturating_add(1);
        if self.model_calls > cfg.max_model_calls_per_turn {
            // The per-turn model-call ceiling is the hard loop guard: it always
            // stops the turn regardless of profile. Interactive turns still emit
            // the softer `RequestContinuation` decision for observability parity.
            return Some(limit_hit(
                ReliabilityErrorClass::ProviderError,
                ReliabilityLimitKind::ModelCallsPerTurn,
                self.model_calls,
                cfg.max_model_calls_per_turn,
                continuation_decision(interactive),
                "model call limit reached",
            ));
        }
        None
    }

    pub(crate) fn record_tool_results(
        &mut self,
        cfg: &RuntimeReliabilityConfig,
        results: &[ToolResultRecord],
        interactive: bool,
    ) -> Option<ReliabilityLimitHit> {
        for result in results {
            if result.is_error {
                self.tool_failures = self.tool_failures.saturating_add(1);
                self.consecutive_tool_failures = self.consecutive_tool_failures.saturating_add(1);
            } else {
                self.consecutive_tool_failures = 0;
            }
        }
        if self.consecutive_tool_failures >= cfg.max_consecutive_tool_failures {
            // Interactive turns already recover from this limit; `continue_on_failure_limit`
            // extends that recovery to non-interactive/eval turns so a short burst of
            // failed tool calls does not end an otherwise-productive turn.
            let decision = continuation_decision(interactive || cfg.continue_on_failure_limit);
            return Some(limit_hit(
                ReliabilityErrorClass::InvalidArguments,
                ReliabilityLimitKind::ConsecutiveToolFailures,
                self.consecutive_tool_failures,
                cfg.max_consecutive_tool_failures,
                decision,
                "consecutive tool failure limit reached",
            ));
        }
        if self.tool_failures >= cfg.max_tool_failures_per_turn {
            // The per-turn total is the hard failure ceiling: always stop so a
            // continuation loop cannot spin forever on a broken tool.
            return Some(limit_hit(
                ReliabilityErrorClass::InvalidArguments,
                ReliabilityLimitKind::ToolFailuresPerTurn,
                self.tool_failures,
                cfg.max_tool_failures_per_turn,
                ReliabilityLimitDecision::StopTurn,
                "tool failure limit reached",
            ));
        }
        None
    }

    /// Clears the consecutive-failure counter after a continuation so the very
    /// next tool round does not immediately re-trip the limit. The per-turn total
    /// (`tool_failures`) is intentionally preserved so the hard ceiling still applies.
    pub(crate) fn reset_consecutive_failures(&mut self) {
        self.consecutive_tool_failures = 0;
    }

    pub(crate) fn tool_failure_count(&self) -> u32 {
        self.tool_failures
    }
}

fn continuation_decision(request_continuation: bool) -> ReliabilityLimitDecision {
    if request_continuation {
        ReliabilityLimitDecision::RequestContinuation
    } else {
        ReliabilityLimitDecision::StopTurn
    }
}

fn limit_hit(
    error_class: ReliabilityErrorClass,
    limit_kind: ReliabilityLimitKind,
    current: u32,
    limit: u32,
    decision: ReliabilityLimitDecision,
    message: &str,
) -> ReliabilityLimitHit {
    ReliabilityLimitHit {
        error_class,
        limit_kind,
        decision,
        current,
        limit,
        message: format!("{message}: {current}/{limit}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: &str, is_error: bool) -> ToolResultRecord {
        ToolResultRecord {
            id: id.to_string(),
            name: Some("test".to_string()),
            result: if is_error { "error" } else { "ok" }.to_string(),
            display_payload: None,
            is_error,
        }
    }

    #[test]
    fn reliability_limits_reset_consecutive_failures_after_success() {
        let cfg = RuntimeReliabilityConfig {
            max_consecutive_tool_failures: 2,
            max_tool_failures_per_turn: 128,
            ..RuntimeReliabilityConfig::default()
        };
        let mut state = TurnReliabilityState::default();

        assert!(
            state
                .record_tool_results(&cfg, &[result("first", true)], false)
                .is_none()
        );
        assert!(
            state
                .record_tool_results(&cfg, &[result("success", false)], false)
                .is_none()
        );
        assert!(
            state
                .record_tool_results(&cfg, &[result("second", true)], false)
                .is_none()
        );
        let limit = state
            .record_tool_results(&cfg, &[result("third", true)], false)
            .unwrap();
        assert_eq!(
            limit.limit_kind,
            ReliabilityLimitKind::ConsecutiveToolFailures
        );
        assert_eq!(limit.current, 2);
    }

    #[test]
    fn consecutive_failure_limit_stops_non_interactive_turn_by_default() {
        let cfg = RuntimeReliabilityConfig {
            max_consecutive_tool_failures: 2,
            ..RuntimeReliabilityConfig::default()
        };
        let mut state = TurnReliabilityState::default();
        state.record_tool_results(&cfg, &[result("first", true)], false);
        let limit = state
            .record_tool_results(&cfg, &[result("second", true)], false)
            .unwrap();
        assert_eq!(
            limit.limit_kind,
            ReliabilityLimitKind::ConsecutiveToolFailures
        );
        assert_eq!(limit.decision, ReliabilityLimitDecision::StopTurn);
    }

    #[test]
    fn consecutive_failure_limit_requests_continuation_when_knob_enabled() {
        let cfg = RuntimeReliabilityConfig {
            max_consecutive_tool_failures: 2,
            continue_on_failure_limit: true,
            ..RuntimeReliabilityConfig::default()
        };
        let mut state = TurnReliabilityState::default();
        state.record_tool_results(&cfg, &[result("first", true)], false);
        let limit = state
            .record_tool_results(&cfg, &[result("second", true)], false)
            .unwrap();
        assert_eq!(
            limit.decision,
            ReliabilityLimitDecision::RequestContinuation
        );

        // Resetting the consecutive counter after a continuation prevents an
        // immediate re-trip while preserving the per-turn total.
        state.reset_consecutive_failures();
        assert!(
            state
                .record_tool_results(&cfg, &[result("third", true)], false)
                .is_none()
        );
    }

    #[test]
    fn tool_failures_per_turn_always_stops_even_with_continuation_knob() {
        let cfg = RuntimeReliabilityConfig {
            max_consecutive_tool_failures: 100,
            max_tool_failures_per_turn: 2,
            continue_on_failure_limit: true,
            ..RuntimeReliabilityConfig::default()
        };
        let mut state = TurnReliabilityState::default();
        state.record_tool_results(&cfg, &[result("first", true)], false);
        let limit = state
            .record_tool_results(&cfg, &[result("second", true)], false)
            .unwrap();
        assert_eq!(limit.limit_kind, ReliabilityLimitKind::ToolFailuresPerTurn);
        assert_eq!(limit.decision, ReliabilityLimitDecision::StopTurn);
    }

    #[test]
    fn default_model_call_limit_allows_long_agentic_turns() {
        assert_eq!(
            RuntimeReliabilityConfig::default().max_model_calls_per_turn,
            512
        );
    }

    #[test]
    fn provider_stream_retry_cause_classifies_transient_stream_failures() {
        assert_eq!(
            provider_stream_retry_cause("error decoding response body"),
            Some("stream_decode_error")
        );
        assert_eq!(
            provider_stream_retry_cause("stream closed before response.completed"),
            Some("stream_closed_before_completed")
        );
        assert_eq!(
            provider_stream_retry_cause("Anthropic stream closed before message_stop"),
            Some("stream_closed_before_message_stop")
        );
        assert_eq!(provider_stream_retry_cause("invalid request body"), None);
    }
}
