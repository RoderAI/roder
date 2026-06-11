//! Hosted identity and tenancy contracts (roadmap phase 72, Task 1).
//!
//! These types are additive for local mode: a locally-run Roder never
//! constructs them. Hosted gateways resolve a `HostedRequestContext` from
//! validated credentials *before* JSON-RPC dispatch; no core thread/turn
//! DTO ever accepts a caller-supplied tenant id.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type TenantId = String;
pub type UserId = String;
pub type ServiceAccountId = String;

/// Who is acting: a human user or a service account. Credential material
/// (tokens, key hashes) never appears here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PrincipalContext {
    User {
        user_id: UserId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
    },
    ServiceAccount {
        service_account_id: ServiceAccountId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
    },
}

impl PrincipalContext {
    pub fn id(&self) -> &str {
        match self {
            PrincipalContext::User { user_id, .. } => user_id,
            PrincipalContext::ServiceAccount {
                service_account_id, ..
            } => service_account_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenantContext {
    pub tenant_id: TenantId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Coarse hosted access scopes attached to a credential.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HostedScope {
    Read,
    Write,
    Admin,
}

/// Tenant-level role of a principal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostedRole {
    Member,
    TenantAdmin,
    /// Operates across tenants (service operators only).
    SystemAdmin,
}

/// Fully resolved request identity, attached by the hosted gateway before
/// dispatch. Local mode never builds one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostedRequestContext {
    pub tenant: TenantContext,
    pub principal: PrincipalContext,
    pub role: HostedRole,
    pub scopes: Vec<HostedScope>,
    /// Stable id of the validated credential (key id / token id), for audit
    /// correlation only — never the secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub authenticated_at: OffsetDateTime,
}

impl HostedRequestContext {
    pub fn has_scope(&self, scope: HostedScope) -> bool {
        self.scopes.contains(&scope)
            || (scope != HostedScope::Admin && self.scopes.contains(&HostedScope::Admin))
    }
}

/// Outcome of an authorization check, carrying a redaction-safe reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum AuthorizationDecision {
    Allow,
    Deny {
        /// Coarse machine-readable reason: `missing_scope`,
        /// `wrong_tenant`, `revoked`, `expired`, `not_member`,
        /// `system_admin_required`.
        reason: String,
    },
}

impl AuthorizationDecision {
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, AuthorizationDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(scopes: Vec<HostedScope>) -> HostedRequestContext {
        HostedRequestContext {
            tenant: TenantContext {
                tenant_id: "tenant-a".to_string(),
                display_name: Some("Tenant A".to_string()),
            },
            principal: PrincipalContext::ServiceAccount {
                service_account_id: "svc-1".to_string(),
                display_name: None,
            },
            role: HostedRole::Member,
            scopes,
            credential_id: Some("key-1".to_string()),
            authenticated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn identity_types_round_trip_without_secret_fields() {
        let context = context(vec![HostedScope::Read, HostedScope::Write]);
        let json = serde_json::to_value(&context).unwrap();
        assert_eq!(json["tenant"]["tenantId"], "tenant-a");
        assert_eq!(json["principal"]["kind"], "service_account");
        assert_eq!(json["scopes"], serde_json::json!(["read", "write"]));
        // The wire shape has no token/secret/key-material fields.
        let text = json.to_string();
        for forbidden in ["token", "secret", "apiKey", "password"] {
            assert!(!text.contains(forbidden), "{text}");
        }
        let round_trip: HostedRequestContext = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, context);
    }

    #[test]
    fn admin_scope_implies_read_and_write() {
        let admin = context(vec![HostedScope::Admin]);
        assert!(admin.has_scope(HostedScope::Read));
        assert!(admin.has_scope(HostedScope::Write));
        assert!(admin.has_scope(HostedScope::Admin));

        let read_only = context(vec![HostedScope::Read]);
        assert!(read_only.has_scope(HostedScope::Read));
        assert!(!read_only.has_scope(HostedScope::Write));
        assert!(!read_only.has_scope(HostedScope::Admin));
    }

    #[test]
    fn authorization_decisions_carry_coarse_reasons() {
        let deny = AuthorizationDecision::deny("wrong_tenant");
        assert!(!deny.is_allowed());
        let json = serde_json::to_value(&deny).unwrap();
        assert_eq!(json["decision"], "deny");
        assert_eq!(json["reason"], "wrong_tenant");
        assert!(AuthorizationDecision::Allow.is_allowed());
    }
}
