//! Hosted hook contracts (roadmap phase 72, Task 1): tenant- or
//! system-scoped event hooks with delivery, retry, and execution-result
//! types. Hook URLs and headers are operator-supplied configuration;
//! signing secrets are referenced by id only and never serialized.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::EventEnvelope;
use crate::identity::{PrincipalContext, TenantContext, TenantId};

pub type HookId = String;
pub type HookDeliveryId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookScope {
    /// Fires only for events belonging to the owning tenant.
    Tenant,
    /// Fires for all tenants (service operators only).
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostedHookDefinition {
    pub id: HookId,
    pub scope: HookScope,
    /// Owning tenant; required for tenant scope, absent for system scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<TenantId>,
    /// Canonical event-kind prefixes this hook subscribes to
    /// (e.g. `turn.`, `thread.`); empty = no events.
    pub event_kinds: Vec<String>,
    /// Delivery target URL (HTTPS in production deployments).
    pub url: String,
    /// Reference to a signing secret managed by the host (`env:NAME` or a
    /// secret-store id). The secret value itself never round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret_ref: Option<String>,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl HostedHookDefinition {
    pub fn matches(&self, event_kind: &str) -> bool {
        self.enabled
            && self
                .event_kinds
                .iter()
                .any(|prefix| event_kind.starts_with(prefix))
    }
}

/// Retry policy for failed deliveries (bounded exponential backoff).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookRetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for HookRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookDeliveryStatus {
    Pending,
    Delivered,
    Failed,
    /// Permanently abandoned after exhausting the retry policy.
    Dead,
}

/// One delivery attempt lifecycle for a matched event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookDelivery {
    pub id: HookDeliveryId,
    pub hook_id: HookId,
    pub event_kind: String,
    pub status: HookDeliveryStatus,
    pub attempts: u32,
    /// Coarse last failure class (`timeout`, `http_4xx`, `http_5xx`,
    /// `connect`); never response bodies or headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub delivered_at: Option<OffsetDateTime>,
}

/// The payload a hook target receives: the canonical envelope plus the
/// hosted actor/tenant context, attached here (rather than on every local
/// event constructor) so local mode stays unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedAuditEnvelope {
    pub envelope: EventEnvelope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<TenantContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalContext>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(scope: HookScope, kinds: Vec<&str>) -> HostedHookDefinition {
        HostedHookDefinition {
            id: "hook-1".to_string(),
            scope,
            tenant_id: matches!(scope, HookScope::Tenant).then(|| "tenant-a".to_string()),
            event_kinds: kinds.into_iter().map(str::to_string).collect(),
            url: "https://hooks.example.test/roder".to_string(),
            signing_secret_ref: Some("env:RODER_HOOK_SECRET".to_string()),
            enabled: true,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn hook_matching_uses_prefixes_and_enabled_flag() {
        let hook = hook(HookScope::Tenant, vec!["turn.", "thread."]);
        assert!(hook.matches("turn.completed"));
        assert!(hook.matches("thread.created"));
        assert!(!hook.matches("tool.call_started"));

        let mut disabled = hook.clone();
        disabled.enabled = false;
        assert!(!disabled.matches("turn.completed"));
    }

    #[test]
    fn hook_definitions_serialize_secret_references_not_secrets() {
        let hook = hook(HookScope::System, vec!["extension."]);
        let json = serde_json::to_value(&hook).unwrap();
        assert_eq!(json["signingSecretRef"], "env:RODER_HOOK_SECRET");
        assert_eq!(json["scope"], "system");
        let round_trip: HostedHookDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, hook);
    }

    #[test]
    fn retry_policy_defaults_are_bounded() {
        let policy = HookRetryPolicy::default();
        assert!(policy.max_attempts >= 1);
        assert!(policy.initial_backoff_ms <= policy.max_backoff_ms);
    }
}
