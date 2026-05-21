use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityErrorClass {
    InvalidArguments,
    UnexpectedEnvironment,
    ProviderError,
    Timeout,
    PolicyDenied,
    UserAborted,
    VerifierFailed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityRetryDecision {
    Retry,
    DoNotRetry,
    Exhausted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityLimitDecision {
    Continue,
    StopTurn,
    RequestContinuation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityLimitKind {
    ConsecutiveToolFailures,
    ToolFailuresPerTurn,
    ModelCallsPerTurn,
    ProviderAttempts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityDetails {
    pub message: String,
    #[serde(default)]
    pub redacted: bool,
}

impl ReliabilityDetails {
    pub fn redacted(message: impl AsRef<str>) -> Self {
        Self {
            message: redact_secret_like_text(message.as_ref()),
            redacted: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityContext {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityFailureRecorded {
    pub context: ReliabilityContext,
    pub error_class: ReliabilityErrorClass,
    pub details: ReliabilityDetails,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityRetryRecorded {
    pub context: ReliabilityContext,
    pub error_class: ReliabilityErrorClass,
    pub decision: ReliabilityRetryDecision,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    pub details: ReliabilityDetails,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityLimitRecorded {
    pub context: ReliabilityContext,
    pub error_class: ReliabilityErrorClass,
    pub limit_kind: ReliabilityLimitKind,
    pub decision: ReliabilityLimitDecision,
    pub current: u32,
    pub limit: u32,
    pub details: ReliabilityDetails,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityMetricRecorded {
    pub context: ReliabilityContext,
    pub metric: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<ReliabilityErrorClass>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityRequestPolicy {
    pub provider_retry_max_attempts: u32,
    pub provider_retry_initial_backoff_ms: u64,
    pub provider_retry_backoff_factor: u32,
    pub retry_empty_provider_body: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_retry_status_codes: Vec<u16>,
}

impl Default for ReliabilityRequestPolicy {
    fn default() -> Self {
        Self {
            provider_retry_max_attempts: 3,
            provider_retry_initial_backoff_ms: 1_000,
            provider_retry_backoff_factor: 2,
            retry_empty_provider_body: true,
            provider_retry_status_codes: vec![429, 500, 502, 503, 504],
        }
    }
}

pub fn provider_retry_delay_ms(policy: &ReliabilityRequestPolicy, attempt: u32) -> u64 {
    let factor = policy.provider_retry_backoff_factor.max(1) as u64;
    policy
        .provider_retry_initial_backoff_ms
        .saturating_mul(factor.saturating_pow(attempt.saturating_sub(1)))
}

pub fn provider_retry_status_cause(status: u16) -> String {
    format!("status_{status}")
}

pub fn provider_retry_metadata(
    attempt: u32,
    cause: &str,
    policy: &ReliabilityRequestPolicy,
) -> Value {
    json!({
        "kind": "reliability_retry_attempt",
        "errorClass": ReliabilityErrorClass::ProviderError,
        "decision": ReliabilityRetryDecision::Retry,
        "attempt": attempt,
        "delayMs": provider_retry_delay_ms(policy, attempt),
        "cause": cause,
    })
}

fn redact_secret_like_text(input: &str) -> String {
    input
        .split_whitespace()
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            if lower.starts_with("sk-")
                || lower.starts_with("bearer")
                || lower.starts_with("authorization:")
                || lower.contains("api_key=")
                || lower.contains("apikey=")
                || lower.contains("token=")
            {
                "[redacted]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventSource, RoderEvent};

    fn context() -> ReliabilityContext {
        ReliabilityContext {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: Some("tool-call-1".to_string()),
            tool_name: Some("read_file".to_string()),
            provider: Some("openai".to_string()),
            model: Some("gpt-5.5".to_string()),
        }
    }

    fn timestamp() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    #[test]
    fn reliability_provider_retry_fixture_serializes_redacted_context() {
        let event = ReliabilityRetryRecorded {
            context: context(),
            error_class: ReliabilityErrorClass::ProviderError,
            decision: ReliabilityRetryDecision::Retry,
            attempt: 1,
            max_attempts: 3,
            delay_ms: Some(1_000),
            details: ReliabilityDetails::redacted(
                "provider 429 Authorization: Bearer sk-secret-token",
            ),
            timestamp: timestamp(),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["errorClass"], "provider_error");
        assert_eq!(json["decision"], "retry");
        assert_eq!(json["context"]["threadId"], "thread-a");
        assert_eq!(json["context"]["turnId"], "turn-a");
        assert_eq!(json["context"]["provider"], "openai");
        assert_eq!(json["context"]["model"], "gpt-5.5");
        let rendered = serde_json::to_string(&json).unwrap();
        assert!(!rendered.contains("sk-secret-token"));
    }

    #[test]
    fn provider_retry_metadata_is_classified_and_redacted() {
        let policy = ReliabilityRequestPolicy {
            provider_retry_initial_backoff_ms: 250,
            provider_retry_backoff_factor: 3,
            ..ReliabilityRequestPolicy::default()
        };

        let metadata = provider_retry_metadata(2, &provider_retry_status_cause(429), &policy);

        assert_eq!(metadata["kind"], "reliability_retry_attempt");
        assert_eq!(metadata["errorClass"], "provider_error");
        assert_eq!(metadata["decision"], "retry");
        assert_eq!(metadata["attempt"], 2);
        assert_eq!(metadata["delayMs"], 750);
        assert_eq!(metadata["cause"], "status_429");
    }

    #[test]
    fn reliability_tool_validation_failure_fixture_serializes() {
        let event = ReliabilityFailureRecorded {
            context: context(),
            error_class: ReliabilityErrorClass::InvalidArguments,
            details: ReliabilityDetails::redacted("missing required field path"),
            timestamp: timestamp(),
        };

        let round_trip: ReliabilityFailureRecorded =
            serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
        assert_eq!(
            round_trip.error_class,
            ReliabilityErrorClass::InvalidArguments
        );
        assert_eq!(round_trip.context.tool_name.as_deref(), Some("read_file"));
    }

    #[test]
    fn reliability_failure_limit_stop_fixture_serializes() {
        let event = ReliabilityLimitRecorded {
            context: context(),
            error_class: ReliabilityErrorClass::InvalidArguments,
            limit_kind: ReliabilityLimitKind::ConsecutiveToolFailures,
            decision: ReliabilityLimitDecision::StopTurn,
            current: 5,
            limit: 5,
            details: ReliabilityDetails::redacted("tool failure limit reached"),
            timestamp: timestamp(),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["limitKind"], "consecutive_tool_failures");
        assert_eq!(json["decision"], "stop_turn");
    }

    #[test]
    fn reliability_timeout_fixture_serializes() {
        let event = ReliabilityMetricRecorded {
            context: context(),
            metric: "timeout_count".to_string(),
            value: 1.0,
            error_class: Some(ReliabilityErrorClass::Timeout),
            timestamp: timestamp(),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["metric"], "timeout_count");
        assert_eq!(json["errorClass"], "timeout");
    }

    #[test]
    fn reliability_unknown_error_fixture_serializes() {
        let event = ReliabilityFailureRecorded {
            context: ReliabilityContext {
                tool_id: None,
                tool_name: None,
                ..context()
            },
            error_class: ReliabilityErrorClass::Unknown,
            details: ReliabilityDetails::redacted("panic converted into unknown harness error"),
            timestamp: timestamp(),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["errorClass"], "unknown");
        assert!(json["context"].get("toolId").is_none());
        assert_eq!(json["context"]["threadId"], "thread-a");
        assert_eq!(json["context"]["turnId"], "turn-a");
    }

    #[test]
    fn reliability_events_expose_kind_source_and_turn_scope() {
        let event = RoderEvent::ReliabilityLimitRecorded(ReliabilityLimitRecorded {
            context: context(),
            error_class: ReliabilityErrorClass::InvalidArguments,
            limit_kind: ReliabilityLimitKind::ToolFailuresPerTurn,
            decision: ReliabilityLimitDecision::RequestContinuation,
            current: 10,
            limit: 10,
            details: ReliabilityDetails::redacted("tool failures per turn reached"),
            timestamp: timestamp(),
        });

        assert_eq!(event.kind(), "reliability.limit");
        assert_eq!(event.source(), EventSource::Core);
        assert_eq!(event.thread_id().map(String::as_str), Some("thread-a"));
        assert_eq!(event.turn_id().map(String::as_str), Some("turn-a"));
    }
}
