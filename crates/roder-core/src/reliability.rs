use roder_api::conversation::ToolResultRecord;
use roder_api::reliability::{
    ReliabilityErrorClass, ReliabilityLimitDecision, ReliabilityLimitKind, ReliabilityRequestPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeReliabilityConfig {
    pub max_consecutive_tool_failures: u32,
    pub max_tool_failures_per_turn: u32,
    pub max_model_calls_per_turn: u32,
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
            max_tool_failures_per_turn: 10,
            max_model_calls_per_turn: 50,
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

impl TurnReliabilityState {
    pub(crate) fn record_model_call(
        &mut self,
        cfg: &RuntimeReliabilityConfig,
        interactive: bool,
    ) -> Option<ReliabilityLimitHit> {
        self.model_calls = self.model_calls.saturating_add(1);
        if self.model_calls > cfg.max_model_calls_per_turn {
            return Some(limit_hit(
                ReliabilityErrorClass::ProviderError,
                ReliabilityLimitKind::ModelCallsPerTurn,
                self.model_calls,
                cfg.max_model_calls_per_turn,
                interactive,
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
            return Some(limit_hit(
                ReliabilityErrorClass::InvalidArguments,
                ReliabilityLimitKind::ConsecutiveToolFailures,
                self.consecutive_tool_failures,
                cfg.max_consecutive_tool_failures,
                interactive,
                "consecutive tool failure limit reached",
            ));
        }
        if self.tool_failures >= cfg.max_tool_failures_per_turn {
            return Some(limit_hit(
                ReliabilityErrorClass::InvalidArguments,
                ReliabilityLimitKind::ToolFailuresPerTurn,
                self.tool_failures,
                cfg.max_tool_failures_per_turn,
                interactive,
                "tool failure limit reached",
            ));
        }
        None
    }
}

fn limit_hit(
    error_class: ReliabilityErrorClass,
    limit_kind: ReliabilityLimitKind,
    current: u32,
    limit: u32,
    interactive: bool,
    message: &str,
) -> ReliabilityLimitHit {
    ReliabilityLimitHit {
        error_class,
        limit_kind,
        decision: if interactive {
            ReliabilityLimitDecision::RequestContinuation
        } else {
            ReliabilityLimitDecision::StopTurn
        },
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
            max_tool_failures_per_turn: 10,
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
}
