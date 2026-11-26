//! Method authorization for hosted requests.
//!
//! Decisions key off the checked-in method manifest's mutation policy:
//! read-only methods need `read`, mutating methods need `write`, and
//! `hosted/*` administration needs `admin` (cross-tenant listing
//! additionally requires the system-admin role). Unknown methods are
//! denied fail-closed.

use roder_api::identity::{AuthorizationDecision, HostedRequestContext, HostedRole, HostedScope};
use roder_protocol::methods::{AppServerSideEffect, app_server_method_specs};

/// Hosted admin methods that operate on the caller's own tenant.
fn is_tenant_admin_method(method: &str) -> bool {
    matches!(
        method,
        "hosted/service_accounts/create"
            | "hosted/service_accounts/revoke"
            | "hosted/hooks/create"
            | "hosted/hooks/update"
            | "hosted/hooks/delete"
            | "hosted/hooks/list"
            | "hosted/hooks/test"
            | "hosted/audit/list"
    )
}

/// Hosted methods that cross tenant boundaries (service operators only).
fn is_system_admin_method(method: &str) -> bool {
    matches!(method, "hosted/tenants/list" | "hosted/tenants/create")
}

pub fn authorize_method(context: &HostedRequestContext, method: &str) -> AuthorizationDecision {
    if method == "thread/resume" && !context.has_scope(HostedScope::Read) {
        return AuthorizationDecision::deny("missing_scope");
    }
    if method == "tools/bind_executor" && !context.has_scope(HostedScope::Read) {
        return AuthorizationDecision::deny("executor_requires_read_scope");
    }
    if method == "hosted/owner/drain" {
        return require_scope(context, HostedScope::Write);
    }
    if method == "hosted/whoami" {
        return AuthorizationDecision::Allow;
    }
    if is_system_admin_method(method) {
        if context.role != HostedRole::SystemAdmin {
            return AuthorizationDecision::deny("system_admin_required");
        }
        return require_scope(context, HostedScope::Admin);
    }
    if is_tenant_admin_method(method) {
        if !matches!(
            context.role,
            HostedRole::TenantAdmin | HostedRole::SystemAdmin
        ) {
            return AuthorizationDecision::deny("tenant_admin_required");
        }
        return require_scope(context, HostedScope::Admin);
    }

    let specs = app_server_method_specs();
    let Some(spec) = specs.iter().find(|spec| spec.method == method) else {
        return AuthorizationDecision::deny("unknown_method");
    };
    let required = match spec.side_effect {
        AppServerSideEffect::ReadOnly => HostedScope::Read,
        AppServerSideEffect::LocalState | AppServerSideEffect::ExternalProcess => {
            HostedScope::Write
        }
    };
    require_scope(context, required)
}

fn require_scope(context: &HostedRequestContext, scope: HostedScope) -> AuthorizationDecision {
    if context.has_scope(scope) {
        AuthorizationDecision::Allow
    } else {
        AuthorizationDecision::deny("missing_scope")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::identity::{PrincipalContext, TenantContext};

    #[test]
    fn thread_resume_requires_read_and_write_scopes() {
        let mut context = HostedRequestContext {
            tenant: TenantContext {
                tenant_id: "tenant".to_string(),
                display_name: None,
            },
            principal: PrincipalContext::User {
                user_id: "user".to_string(),
                display_name: None,
            },
            role: HostedRole::Member,
            scopes: vec![HostedScope::Read],
            credential_id: None,
            authenticated_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        assert_eq!(
            authorize_method(&context, "thread/history/read"),
            AuthorizationDecision::Allow
        );
        assert_eq!(
            authorize_method(&context, "thread/resume"),
            AuthorizationDecision::deny("missing_scope")
        );
        context.scopes = vec![HostedScope::Write];
        assert_eq!(
            authorize_method(&context, "thread/resume"),
            AuthorizationDecision::deny("missing_scope")
        );
        context.scopes.push(HostedScope::Read);
        assert_eq!(
            authorize_method(&context, "thread/resume"),
            AuthorizationDecision::Allow
        );
    }
}
