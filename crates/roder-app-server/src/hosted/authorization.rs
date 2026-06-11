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
