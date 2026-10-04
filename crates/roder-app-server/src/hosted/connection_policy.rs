//! Deployment policy and bounded live authority checks for hosted sockets.
use std::time::Duration;

use futures::future::BoxFuture;
use roder_api::identity::HostedRequestContext;
use roder_protocol::JsonRpcRequest;
use time::OffsetDateTime;

use super::{HostedAuthenticator, TenantRegistry};

/// Result of applying deployment-specific hosted request policy.
#[derive(Debug, Clone)]
pub enum HostedRequestPolicyDecision {
    /// Dispatch this request after the gateway's built-in authorization and
    /// workspace checks. The request may differ from the original.
    Allow(JsonRpcRequest),
    /// Reject the request with a JSON-RPC forbidden response.
    Deny { reason: String },
}

impl HostedRequestPolicyDecision {
    /// Allows a request, optionally after rewriting it.
    pub fn allow(request: JsonRpcRequest) -> Self {
        Self::Allow(request)
    }

    /// Denies a request with an audit-safe reason code or message.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }
}

/// Applies deployment-specific policy to authenticated hosted requests.
///
/// The bearer is provided so a host can bind request capabilities to the
/// authenticated connection. Implementations must not log or persist it.
pub trait HostedRequestPolicy: Send + Sync {
    /// Rechecks live deployment authority at admission, before dispatch and
    /// while idle. Denial closes this socket and terminalizes its executors;
    /// it does not affect another connection or cancel unrelated tenant work.
    /// The gateway bounds this future to five seconds and redacts the bearer
    /// from failures. Implementations must not log or persist the bearer.
    fn revalidate<'a>(
        &'a self,
        _context: &'a HostedRequestContext,
        _bearer_token: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    /// Idle revalidation cadence, clamped by the gateway to 1–60 seconds.
    /// Credentials are still validated before every parsed request.
    fn revalidation_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Inspects, rewrites, or denies a request before JSON-RPC dispatch.
    fn evaluate(
        &self,
        context: &HostedRequestContext,
        bearer_token: &str,
        request: JsonRpcRequest,
    ) -> HostedRequestPolicyDecision;
}

/// Default hosted request policy that leaves every request unchanged.
#[derive(Debug, Default)]
pub struct AllowAllHostedRequestPolicy;

impl HostedRequestPolicy for AllowAllHostedRequestPolicy {
    fn evaluate(
        &self,
        _context: &HostedRequestContext,
        _bearer_token: &str,
        request: JsonRpcRequest,
    ) -> HostedRequestPolicyDecision {
        HostedRequestPolicyDecision::Allow(request)
    }
}

pub(super) fn redact_bearer(reason: &str, bearer_token: &str) -> String {
    reason.replace(bearer_token, "[REDACTED]")
}

pub(super) async fn revalidate_connection(
    authenticator: &HostedAuthenticator,
    tenants: &TenantRegistry,
    policy: &dyn HostedRequestPolicy,
    established: &HostedRequestContext,
    bearer_token: &str,
) -> Result<HostedRequestContext, String> {
    let mut revalidated = authenticator
        .authenticate(bearer_token, tenants, OffsetDateTime::now_utc())
        .map_err(|error| redact_bearer(&error.to_string().replace(' ', "_"), bearer_token))?;
    revalidated.credential_id = revalidated
        .credential_id
        .map(|id| redact_bearer(&id, bearer_token));
    if !same_connection_identity(established, &revalidated) {
        return Err("credential_identity_changed".to_string());
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        policy.revalidate(&revalidated, bearer_token),
    )
    .await
    .map_err(|_| "connection_authorization_timeout".to_string())?
    .map_err(|reason| redact_bearer(&reason, bearer_token))?;
    // The refreshed authentication timestamp is intentionally updated;
    // identity, role, and scopes remain bound to this tenant connection.
    Ok(revalidated)
}

fn same_connection_identity(
    established: &HostedRequestContext,
    revalidated: &HostedRequestContext,
) -> bool {
    established.tenant.tenant_id == revalidated.tenant.tenant_id
        && established.principal == revalidated.principal
        && established.role == revalidated.role
        && established.scopes == revalidated.scopes
        && established.credential_id == revalidated.credential_id
}
