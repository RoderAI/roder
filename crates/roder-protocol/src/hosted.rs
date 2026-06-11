//! `hosted/*` protocol DTOs (roadmap phase 72, Task 1).
//!
//! Contract-first: these DTOs and the method manifest entries define the
//! hosted multi-tenant surface before the gateway implementation lands
//! (phase 72 Task 2). Tenant identity is always resolved server-side from
//! validated credentials — no request DTO here accepts a caller-supplied
//! tenant id for core operations.

use serde::{Deserialize, Serialize};

pub use roder_api::hosted_hooks::{
    HookDelivery, HookDeliveryStatus, HookId, HookRetryPolicy, HookScope, HostedHookDefinition,
};
pub use roder_api::identity::{
    HostedRequestContext, HostedRole, HostedScope, PrincipalContext, ServiceAccountId,
    TenantContext, TenantId,
};

/// `hosted/whoami` — the resolved identity of the calling credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedWhoamiResult {
    pub context: HostedRequestContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedTenantsListResult {
    /// Tenants the principal belongs to (system admins see all).
    pub tenants: Vec<TenantContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedTenantReadParams {
    /// Admin-surface read of a tenant the caller can administer; absent =
    /// the caller's own tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<TenantId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedTenantReadResult {
    pub tenant: TenantContext,
    pub member_count: u64,
    pub service_account_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountSummary {
    pub service_account_id: ServiceAccountId,
    pub display_name: String,
    pub scopes: Vec<HostedScope>,
    /// Stable credential id for audit correlation; never key material.
    pub credential_id: String,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountsListResult {
    pub service_accounts: Vec<HostedServiceAccountSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountCreateParams {
    pub display_name: String,
    pub scopes: Vec<HostedScope>,
}

/// The API key secret appears exactly once, in this response; the server
/// stores only its hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountCreateResult {
    pub service_account: HostedServiceAccountSummary,
    pub api_key_once: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountRevokeParams {
    pub service_account_id: ServiceAccountId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedServiceAccountRevokeResult {
    pub service_account_id: ServiceAccountId,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHooksListResult {
    pub hooks: Vec<HostedHookDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHookCreateParams {
    pub scope: HookScope,
    pub event_kinds: Vec<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret_ref: Option<String>,
    #[serde(default)]
    pub retry: Option<HookRetryPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHookUpdateParams {
    pub hook_id: HookId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_kinds: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHookDeleteParams {
    pub hook_id: HookId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHookResult {
    pub hook: HostedHookDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedAuditListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// A redaction-safe audit row: coarse action, actor/tenant context, and an
/// outcome — never request bodies or credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedAuditRecord {
    pub action: String,
    pub outcome: String,
    pub tenant_id: TenantId,
    pub principal: PrincipalContext,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedAuditListResult {
    pub records: Vec<HostedAuditRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedUsageReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedUsageReadResult {
    pub tenant_id: TenantId,
    pub turn_count: u64,
    pub tool_call_count: u64,
    pub total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn hosted_dtos_use_camel_case_and_round_trip() {
        let whoami = HostedWhoamiResult {
            context: HostedRequestContext {
                tenant: TenantContext {
                    tenant_id: "tenant-a".to_string(),
                    display_name: None,
                },
                principal: PrincipalContext::User {
                    user_id: "user-1".to_string(),
                    display_name: Some("Dev".to_string()),
                },
                role: HostedRole::TenantAdmin,
                scopes: vec![HostedScope::Admin],
                credential_id: Some("key-9".to_string()),
                authenticated_at: OffsetDateTime::UNIX_EPOCH,
            },
        };
        let json = serde_json::to_value(&whoami).unwrap();
        assert_eq!(json["context"]["tenant"]["tenantId"], "tenant-a");
        assert_eq!(json["context"]["role"], "tenant_admin");

        // Core thread/turn DTOs never accept tenant ids; only hosted admin
        // DTOs reference tenants, and only for admin reads.
        let params: HostedTenantReadParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(params.tenant_id.is_none());

        let create: HostedHookCreateParams = serde_json::from_value(serde_json::json!({
            "scope": "tenant",
            "eventKinds": ["turn."],
            "url": "https://hooks.example.test/x"
        }))
        .unwrap();
        assert_eq!(create.scope, HookScope::Tenant);
        assert!(create.retry.is_none());
    }
}
