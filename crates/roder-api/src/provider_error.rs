//! Typed provider failures shared by transports and the sampling loop.
use std::fmt;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureKind {
    Transport,
    StreamInterrupted,
    RateLimited,
    ServerOverloaded,
    ContextWindowExceeded,
    UsageLimit,
    QuotaExceeded,
    Authentication,
    InvalidRequest,
    Protocol,
    ToolSearchExhausted,
    RequestTooLarge,
}

impl ProviderFailureKind {
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Transport | Self::StreamInterrupted | Self::RateLimited | Self::ServerOverloaded
        )
    }

    pub fn retry_cause(self) -> &'static str {
        match self {
            Self::Transport => "transport_error",
            Self::StreamInterrupted => "stream_interrupted",
            Self::RateLimited => "rate_limit",
            Self::ServerOverloaded => "server_overloaded",
            Self::ContextWindowExceeded => "context_window_exceeded",
            Self::UsageLimit => "usage_limit",
            Self::QuotaExceeded => "quota_exceeded",
            Self::Authentication => "authentication",
            Self::InvalidRequest => "invalid_request",
            Self::Protocol => "protocol_error",
            Self::ToolSearchExhausted => "tool_search_exhausted",
            Self::RequestTooLarge => "request_too_large",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderFailure {
    pub kind: ProviderFailureKind,
    pub message: String,
    pub provider_code: Option<String>,
    pub status: Option<u16>,
    pub request_id: Option<String>,
    pub response_id: Option<String>,
    /// Absolute monotonic deadline captured on receipt of provider advice.
    pub retry_not_before: Option<Instant>,
    /// Transport already spent this sampling step's request retry budget.
    pub retry_budget_exhausted: bool,
}

impl ProviderFailure {
    pub fn metadata(&self) -> serde_json::Value {
        serde_json::json!({"kind":"provider_failure","cause":self.kind.retry_cause(),
            "providerCode":self.provider_code,"status":self.status,"requestId":self.request_id,
            "responseId":self.response_id,"retryable":self.kind.is_retryable(),
            "retryBudgetExhausted":self.retry_budget_exhausted,
            "retryAfterMs":self.retry_not_before.map(|deadline|deadline.saturating_duration_since(Instant::now()).as_millis().min(u64::MAX as u128) as u64)})
    }

    pub fn new(kind: ProviderFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            provider_code: None,
            status: None,
            request_id: None,
            response_id: None,
            retry_not_before: None,
            retry_budget_exhausted: false,
        }
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for ProviderFailure {}
