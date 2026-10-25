//! Bounded, process-local follow-ups for an existing harness thread.
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadEventSubscribeRequest {
    pub topic: String,
    pub prompt: String,
    /// Omit both delay and interval for a publish-only subscription.
    pub delay_seconds: Option<u64>,
    pub interval_seconds: Option<u64>,
    #[serde(default = "default_max_deliveries")]
    pub max_deliveries: u32,
    #[serde(default = "default_ttl_seconds")]
    pub ttl_seconds: u64,
}

fn default_max_deliveries() -> u32 {
    100
}
fn default_ttl_seconds() -> u64 {
    86_400
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadEventSubscription {
    pub id: String,
    pub thread_id: String,
    pub topic: String,
    pub prompt: String,
    pub interval_seconds: Option<u64>,
    pub remaining_deliveries: u32,
    /// Next timer deadline; absent for publish-only subscriptions or a pending one-shot.
    #[serde(with = "time::serde::rfc3339::option")]
    pub next_tick_at: Option<OffsetDateTime>,
    /// Last admitted follow-up turn, regardless of its eventual completion status.
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_delivered_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub pending: bool,
    pub last_error: Option<String>,
}
