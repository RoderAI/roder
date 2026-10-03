//! Hosted multi-tenant WebSocket gateway.
//!
//! Every connection authenticates with a bearer credential in the
//! `Authorization` header or browser-compatible WebSocket subprotocol at
//! handshake time (query-string credentials are always rejected) and resolves
//! to a `HostedRequestContext` before any JSON-RPC dispatch. The credential is
//! revalidated before every parsed request so expiry, revocation, and external
//! verifier decisions take effect on already-open sockets. Per request the
//! gateway then enforces, in order: frame size, rate limit, deployment policy,
//! method authorization, and hosted workspace policy; only then does the
//! tenant's app-server see the request.
//!
//! Tenancy is runtime-per-tenant (`HostedRuntimePool`): each connection
//! resolves its tenant's own `AppServer` (own thread/artifact/automation
//! stores, own notification stream), so cross-tenant listing, reads,
//! mutations, and notifications are impossible by construction — tenant
//! ids come from the authenticated context only, never from payloads.
//! `hosted/*` administration is handled in the gateway itself. A resident
//! connection holds a tenant lease so its runtime is never idle-evicted
//! mid-session.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use roder_api::identity::{AuthorizationDecision, HostedRequestContext, HostedRole, HostedScope};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use time::OffsetDateTime;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};

use super::audit::{AuditLog, AuditRecord};
use super::auth::{HostedAuthenticator, PrincipalSeed};
use super::authorization::authorize_method;
use super::hook_delivery::HookDeliveryService;
use super::hooks::HookStore;
use super::rate_limit::{RateLimitConfig, RateLimiter};
use super::runtime_pool::HostedRuntimePool;
use super::tenant::TenantRegistry;
use crate::remote::REMOTE_PROTOCOL;

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

pub struct HostedGatewayOptions {
    pub listen: String,
    pub authenticator: Arc<HostedAuthenticator>,
    pub tenants: Arc<TenantRegistry>,
    pub audit: Arc<AuditLog>,
    pub limits: RateLimitConfig,
    pub hooks: Arc<HookStore>,
    pub hook_delivery: Arc<HookDeliveryService>,
    pub request_policy: Arc<dyn HostedRequestPolicy>,
}

pub struct HostedGatewayController {
    pub listen_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl HostedGatewayController {
    pub async fn stop(mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await??;
        Ok(())
    }
}

pub async fn serve_hosted_gateway(
    pool: Arc<HostedRuntimePool>,
    options: HostedGatewayOptions,
) -> anyhow::Result<HostedGatewayController> {
    let listener = TcpListener::bind(&options.listen).await?;
    let listen_addr = listener.local_addr()?;
    let limiter = Arc::new(RateLimiter::new(options.limits));
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        let mut idle_eviction =
            tokio::time::interval(idle_eviction_interval(pool.profile().idle_ttl));
        idle_eviction.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // `interval`'s first tick is immediate; consume it so an unused pool
        // is not scanned before one full bounded interval has elapsed.
        idle_eviction.tick().await;
        loop {
            let accepted = tokio::select! {
                biased;
                _ = &mut shutdown_rx => break,
                _ = idle_eviction.tick() => {
                    pool.evict_idle().await;
                    continue;
                }
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _peer)) = accepted else {
                break;
            };
            let pool = pool.clone();
            let authenticator = options.authenticator.clone();
            let tenants = options.tenants.clone();
            let audit = options.audit.clone();
            let limiter = limiter.clone();
            let hooks = options.hooks.clone();
            let hook_delivery = options.hook_delivery.clone();
            let request_policy = options.request_policy.clone();
            connections.spawn(async move {
                serve_connection(
                    pool,
                    authenticator,
                    tenants,
                    audit,
                    limiter,
                    hooks,
                    hook_delivery,
                    request_policy,
                    stream,
                )
                .await;
            });
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        pool.drain_on_shutdown().await
    });

    Ok(HostedGatewayController {
        listen_addr,
        shutdown: Some(shutdown_tx),
        task,
    })
}

const MIN_IDLE_EVICTION_INTERVAL: Duration = Duration::from_millis(50);
const MAX_IDLE_EVICTION_INTERVAL: Duration = Duration::from_secs(60);
const AUTH_REVALIDATION_INTERVAL: Duration = Duration::from_secs(1);

/// Scans often enough to make the configured TTL meaningful while bounding
/// both zero-TTL spin loops and work done by long-lived hosted gateways.
fn idle_eviction_interval(idle_ttl: Duration) -> Duration {
    (idle_ttl / 2).clamp(MIN_IDLE_EVICTION_INTERVAL, MAX_IDLE_EVICTION_INTERVAL)
}

#[allow(clippy::too_many_arguments)]
async fn serve_connection(
    pool: Arc<HostedRuntimePool>,
    authenticator: Arc<HostedAuthenticator>,
    tenants: Arc<TenantRegistry>,
    audit: Arc<AuditLog>,
    limiter: Arc<RateLimiter>,
    hooks: Arc<HookStore>,
    hook_delivery: Arc<HookDeliveryService>,
    request_policy: Arc<dyn HostedRequestPolicy>,
    mut stream: tokio::net::TcpStream,
) {
    if respond_to_health_probe(&mut stream, &pool).await {
        return;
    }
    // Authenticate at handshake time, before any request dispatch.
    let authentication: Arc<Mutex<Option<AuthenticatedConnection>>> = Arc::default();
    let callback_authentication = authentication.clone();
    let callback_audit = audit.clone();
    let callback_authenticator = authenticator.clone();
    let callback_tenants = tenants.clone();
    #[allow(clippy::result_large_err)]
    let callback = move |request: &Request,
                         mut response: Response|
          -> Result<Response, ErrorResponse> {
        let deny = |reason: &str| {
            callback_audit.record(AuditRecord {
                kind: "auth_failed".to_string(),
                tenant_id: None,
                principal_id: None,
                credential_id: None,
                method: None,
                reason: Some(reason.to_string()),
                timestamp: OffsetDateTime::now_utc(),
            });
            let mut error = ErrorResponse::new(Some(reason.to_string()));
            *error.status_mut() = StatusCode::UNAUTHORIZED;
            error
        };
        // Credentials in query strings are forbidden, full stop.
        if request
            .uri()
            .query()
            .is_some_and(|query| query.to_ascii_lowercase().contains("token"))
        {
            return Err(deny("credentials_in_query"));
        }
        let Some(bearer) = bearer_from_request(request) else {
            return Err(deny("missing_credential"));
        };
        if bearer.source == BearerSource::Subprotocol && !request_supports_remote_protocol(request)
        {
            return Err(deny("missing_remote_subprotocol"));
        }
        match callback_authenticator.authenticate(
            bearer.token,
            &callback_tenants,
            OffsetDateTime::now_utc(),
        ) {
            Ok(mut resolved) => {
                if request.headers().contains_key(super::owner_routing::OWNER_HOP_HEADER) {
                    let expected = super::owner_routing::tenant_digest(&resolved.tenant.tenant_id);
                    if request.headers().get(super::owner_routing::OWNER_TENANT_HEADER)
                        .and_then(|value| value.to_str().ok()) != Some(expected.as_str()) {
                        return Err(deny("owner_tenant_mismatch"));
                    }
                }
                resolved.credential_id = resolved
                    .credential_id
                    .map(|id| redact_bearer(&id, bearer.token));
                callback_audit.record(AuditRecord {
                    kind: "auth_ok".to_string(),
                    tenant_id: Some(resolved.tenant.tenant_id.clone()),
                    principal_id: Some(resolved.principal.id().to_string()),
                    credential_id: resolved.credential_id.clone(),
                    method: None,
                    reason: None,
                    timestamp: OffsetDateTime::now_utc(),
                });
                *callback_authentication.lock().unwrap() = Some(AuthenticatedConnection {
                    context: resolved,
                    bearer_token: bearer.token.to_string(),
                    forwarded: request.headers().contains_key(super::owner_routing::OWNER_HOP_HEADER),
                });
                if request_supports_remote_protocol(request) {
                    response.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        HeaderValue::from_static(REMOTE_PROTOCOL),
                    );
                }
                Ok(response)
            }
            Err(error) => Err(deny(&error.to_string().replace(' ', "_"))),
        }
    };

    let Ok(mut websocket) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let Some(authentication) = authentication.lock().unwrap().clone() else {
        return;
    };
    let mut context = authentication.context;
    let bearer_token = authentication.bearer_token;

    // Resolve the tenant's runtime and hold the lease for the lifetime of
    // the connection so it is never idle-evicted mid-session.
    let lease = match pool.lease(&context.tenant.tenant_id).await {
        Ok(lease) => lease,
        Err(error) => {
            if let Some(route) = error.downcast_ref::<super::owner_routing::HostedRuntimeRedirect>() {
                let reason = if authentication.forwarded {
                    let _ = websocket.close(None).await;
                    Some("owner_route_hop_limit")
                } else if route.tenant_id != context.tenant.tenant_id {
                    let _ = websocket.close(None).await;
                    Some("owner_route_tenant_mismatch")
                } else {
                    // The owning gateway authenticates again and applies its
                    // policy. A forwarded connection can never forward again.
                    match pool.admit_owner_relay().await {
                        Ok(_relay) => super::owner_routing::relay_to_owner(
                            websocket, route.endpoint, &bearer_token, &context.tenant.tenant_id,
                        ).await.err().map(|_| "owner_connection_unavailable"),
                        Err(_) => {
                            let _ = websocket.close(None).await;
                            Some("owner_relay_draining")
                        }
                    }
                };
                if let Some(reason) = reason {
                    audit.record(AuditRecord {
                        kind: "owner_routing_failed".to_string(),
                        tenant_id: Some(context.tenant.tenant_id.clone()),
                        principal_id: Some(context.principal.id().to_string()),
                        credential_id: context.credential_id.clone(),
                        method: None,
                        reason: Some(reason.to_string()),
                        timestamp: OffsetDateTime::now_utc(),
                    });
                }
                return;
            }
            let _ = websocket.close(None).await;
            audit.record(AuditRecord {
                kind: "runtime_unavailable".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: context.credential_id.clone(),
                method: None,
                reason: Some(format!("tenant runtime failed to start: {error}")),
                timestamp: OffsetDateTime::now_utc(),
            });
            return;
        }
    };
    let app_server = lease.server.clone();
    let mut executor_connection = super::executor_gateway::ExecutorConnection::new(app_server.clone());

    let (mut ws_write, mut ws_read) = websocket.split();
    let connection_authorized = Arc::new(AtomicBool::new(true));
    let writer_authorized = connection_authorized.clone();
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<OutboundMessage>();
    let writer_server = app_server.clone();
    let mut writer_tasks = tokio::task::JoinSet::new();
    writer_tasks.spawn(async move {
        while let Some(outbound) = outbound_rx.recv().await {
            if writer_server.runtime.ensure_execution_authority().is_err() {
                writer_authorized.store(false, Ordering::Release);
                let _ = ws_write.send(Message::Close(None)).await;
                break;
            }
            let message = match outbound {
                OutboundMessage::Control(message) => message,
                OutboundMessage::Notification(message)
                    if writer_authorized.load(Ordering::Acquire) =>
                {
                    message
                }
                OutboundMessage::Notification(_) => continue,
            };
            if ws_write.send(message).await.is_err() {
                break;
            }
        }
    });

    // Notifications come from the tenant's own app-server, so everything on
    // this stream already belongs to this tenant; the read scope gates the
    // subscription itself.
    let mut notification_tasks = tokio::task::JoinSet::new();
    if context.has_scope(HostedScope::Read) {
        let mut notifications = app_server.subscribe_notifications();
        let notification_tx = outbound_tx.clone();
        let executor_server = app_server.clone();
        let executor_id = executor_connection.id.clone();
        notification_tasks.spawn(async move {
            loop {
                let notification = match notifications.recv().await {
                    Ok(notification) => notification,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        for revoked in executor_server.external_tool_executors.revoke_connection(&executor_id, "uncertain").await {
                            super::executor_gateway::revoke(&executor_server, revoked, "notification_lag").await;
                        }
                        let _ = notification_tx.send(OutboundMessage::Control(Message::Close(None)));
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let notification = match executor_server
                    .external_tool_executors
                    .observe(&executor_id, notification)
                    .await
                {
                    super::external_tools::Delivery::Send(notification) => notification,
                    super::external_tools::Delivery::Suppress => continue,
                    super::external_tools::Delivery::Reject(request) => {
                        super::executor_gateway::reject_execution(
                            &executor_server,
                            &request,
                            "executor_unavailable",
                        )
                        .await;
                        continue;
                    }
                };
                let Ok(text) = serde_json::to_string(&notification) else {
                    continue;
                };
                if notification_tx
                    .send(OutboundMessage::Notification(Message::Text(text.into())))
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    // Revalidate even when a client is completely idle. Otherwise an expired
    // or revoked socket could keep consuming tenant notifications forever by
    // never sending another request.
    let mut auth_revalidation = tokio::time::interval(AUTH_REVALIDATION_INTERVAL);
    auth_revalidation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    auth_revalidation.tick().await;

    'connection: loop {
        if app_server.runtime.ensure_execution_authority().is_err() {
            connection_authorized.store(false, Ordering::Release);
            let _ = outbound_tx.send(OutboundMessage::Control(Message::Close(None)));
            break;
        }
        let message = tokio::select! {
            _ = auth_revalidation.tick() => {
                match revalidate_connection(
                    &authenticator,
                    &tenants,
                    &context,
                    &bearer_token,
                ) {
                    Ok(revalidated) => context = revalidated,
                    Err(reason) => {
                        audit.record(AuditRecord {
                            kind: "auth_revalidation_failed".to_string(),
                            tenant_id: Some(context.tenant.tenant_id.clone()),
                            principal_id: Some(context.principal.id().to_string()),
                            credential_id: context.credential_id.clone(),
                            method: None,
                            reason: Some(reason.clone()),
                            timestamp: OffsetDateTime::now_utc(),
                        });
                        // Mark the connection invalid before aborting the
                        // producer. The writer drops any notification already
                        // queued ahead of the terminal error and close frame.
                        connection_authorized.store(false, Ordering::Release);
                        notification_tasks.abort_all();
                        send_error(
                            &outbound_tx,
                            serde_json::Value::Null,
                            -32013,
                            &format!("authentication no longer valid: {reason}"),
                        );
                        let _ = outbound_tx
                            .send(OutboundMessage::Control(Message::Close(None)));
                        break 'connection;
                    }
                }
                continue;
            }
            message = ws_read.next() => match message {
                Some(Ok(message)) => message,
                _ => break 'connection,
            },
        };
        if app_server.runtime.ensure_execution_authority().is_err() {
            connection_authorized.store(false, Ordering::Release);
            let _ = outbound_tx.send(OutboundMessage::Control(Message::Close(None)));
            break;
        }
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break 'connection,
            _ => continue,
        };
        if text.len() > limiter.max_request_bytes() {
            send_error(
                &outbound_tx,
                serde_json::Value::Null,
                -32600,
                "request too large",
            );
            continue;
        }
        let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&text) else {
            send_error(&outbound_tx, serde_json::Value::Null, -32700, "parse error");
            continue;
        };
        let id = request.id.clone().unwrap_or(serde_json::Value::Null);
        let requested_method = request.method.clone();

        // A valid handshake does not grant an unbounded session: external
        // JWT/session verifiers, expiring service-account keys, and revoked
        // service-account keys are checked again before every dispatch.
        context = match revalidate_connection(&authenticator, &tenants, &context, &bearer_token) {
            Ok(revalidated) => revalidated,
            Err(reason) => {
                audit.record(AuditRecord {
                    kind: "auth_revalidation_failed".to_string(),
                    tenant_id: Some(context.tenant.tenant_id.clone()),
                    principal_id: Some(context.principal.id().to_string()),
                    credential_id: context.credential_id.clone(),
                    method: Some(requested_method),
                    reason: Some(reason.clone()),
                    timestamp: OffsetDateTime::now_utc(),
                });
                // Prevent queued tenant events from racing ahead of the
                // terminal auth error once this failure is known.
                connection_authorized.store(false, Ordering::Release);
                notification_tasks.abort_all();
                send_error(
                    &outbound_tx,
                    id,
                    -32013,
                    &format!("authentication no longer valid: {reason}"),
                );
                let _ = outbound_tx.send(OutboundMessage::Control(Message::Close(None)));
                break 'connection;
            }
        };

        if !limiter.check(
            &context.tenant.tenant_id,
            context.principal.id(),
            OffsetDateTime::now_utc(),
        ) {
            audit.record(AuditRecord {
                kind: "rate_limited".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: context.credential_id.clone(),
                method: Some(request.method.clone()),
                reason: None,
                timestamp: OffsetDateTime::now_utc(),
            });
            send_error(&outbound_tx, id, -32011, "rate limit exceeded");
            continue;
        }

        let request = match request_policy.evaluate(&context, &bearer_token, request) {
            HostedRequestPolicyDecision::Allow(request) => request,
            HostedRequestPolicyDecision::Deny { reason } => {
                let safe_reason = redact_bearer(&reason, &bearer_token);
                audit.record(AuditRecord {
                    kind: "request_policy_denied".to_string(),
                    tenant_id: Some(context.tenant.tenant_id.clone()),
                    principal_id: Some(context.principal.id().to_string()),
                    credential_id: context.credential_id.clone(),
                    method: Some(requested_method),
                    reason: Some(safe_reason.clone()),
                    timestamp: OffsetDateTime::now_utc(),
                });
                send_error(
                    &outbound_tx,
                    id,
                    -32012,
                    &format!("forbidden: {safe_reason}"),
                );
                continue;
            }
        };

        if let AuthorizationDecision::Deny { reason } = authorize_method(&context, &request.method)
        {
            audit.record(AuditRecord {
                kind: "method_denied".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: context.credential_id.clone(),
                method: Some(request.method.clone()),
                reason: Some(reason.clone()),
                timestamp: OffsetDateTime::now_utc(),
            });
            send_error(&outbound_tx, id, -32012, &format!("forbidden: {reason}"));
            continue;
        }

        // Hosted workspace policy: tenants must not execute against
        // arbitrary host paths unless the profile opted into local
        // workspaces (test/single-box profiles only).
        if !pool.profile().allow_local_workspaces && denies_local_workspace(&request) {
            audit.record(AuditRecord {
                kind: "method_denied".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: context.credential_id.clone(),
                method: Some(request.method.clone()),
                reason: Some("local_workspace_disabled".to_string()),
                timestamp: OffsetDateTime::now_utc(),
            });
            send_error(
                &outbound_tx,
                id,
                -32012,
                "forbidden: local_workspace_disabled (hosted execution requires a configured \
                 runner destination)",
            );
            continue;
        }

        let _request_admission = match pool.admit_request(&context.tenant.tenant_id, &request.method).await {
            Ok(admission) => admission,
            Err(_) => {
                send_error(&outbound_tx, id, -32015, "hosted replica is draining; request was not dispatched");
                continue;
            }
        };
        let response = if let Some(response) =
            super::executor_gateway::dispatch(&app_server, &executor_connection.id, &request).await
        {
            response
        } else if request.method.starts_with("hosted/") {
            handle_hosted_method(&context, &authenticator, &tenants, &audit, &hooks, request)
        } else {
            let method = request.method.clone();
            let response = app_server.handle_request(request).await;
            // Dispatch matching tenant hooks for lifecycle events
            // (fire-and-forget; failures are bounded by the delivery
            // service and recorded as redacted delivery records).
            if response.error.is_none()
                && let Some(event_kind) = hook_event_kind(&method)
            {
                let thread_id = response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("thread"))
                    .and_then(|thread| thread.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let payload = serde_json::json!({
                    "kind": event_kind,
                    "tenantId": context.tenant.tenant_id,
                    "threadId": thread_id,
                });
                for hook in hooks.matching(&context.tenant.tenant_id, event_kind) {
                    let delivery = hook_delivery.clone();
                    let payload = payload.clone();
                    let kind = event_kind.to_string();
                    tokio::spawn(async move {
                        let _ = delivery.deliver(&hook, &kind, &payload).await;
                    });
                }
            }
            response
        };
        if let Ok(text) = serde_json::to_string(&response) {
            let _ = outbound_tx.send(OutboundMessage::Control(Message::Text(text.into())));
        }
    }
    executor_connection.close().await;
    // Stop tenant notifications first, then close the outbound channel and
    // let the writer drain any final authentication error + close frame.
    notification_tasks.abort_all();
    while notification_tasks.join_next().await.is_some() {}
    drop(outbound_tx);
    while writer_tasks.join_next().await.is_some() {}
    drop(lease);
}

#[derive(Clone)]
struct AuthenticatedConnection {
    context: HostedRequestContext,
    bearer_token: String,
    forwarded: bool,
}

enum OutboundMessage {
    /// Responses and terminal close frames that must still drain after an auth
    /// failure so the client receives a useful, redacted reason.
    Control(Message),
    /// Tenant events that are discarded as soon as the connection loses
    /// authorization, including events already queued by the subscriber task.
    Notification(Message),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BearerSource {
    Authorization,
    Subprotocol,
}

#[derive(Debug, Clone, Copy)]
struct BearerCredential<'a> {
    token: &'a str,
    source: BearerSource,
}

fn bearer_from_request(request: &Request) -> Option<BearerCredential<'_>> {
    request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
        .map(|token| BearerCredential {
            token,
            source: BearerSource::Authorization,
        })
        .or_else(|| {
            request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    value.split(',').map(str::trim).find_map(|part| {
                        part.strip_prefix("bearer.")
                            .filter(|token| !token.is_empty())
                            .map(|token| BearerCredential {
                                token,
                                source: BearerSource::Subprotocol,
                            })
                    })
                })
        })
}

fn request_supports_remote_protocol(request: &Request) -> bool {
    request
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|part| part == REMOTE_PROTOCOL)
        })
}

fn redact_bearer(reason: &str, bearer_token: &str) -> String {
    reason.replace(bearer_token, "[REDACTED]")
}

fn revalidate_connection(
    authenticator: &HostedAuthenticator,
    tenants: &TenantRegistry,
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

async fn respond_to_health_probe(stream: &mut tokio::net::TcpStream, pool: &HostedRuntimePool) -> bool {
    let mut buffer = [0_u8; 512];
    let Ok(bytes_read) = stream.peek(&mut buffer).await else {
        return false;
    };
    if !is_health_probe(&buffer[..bytes_read]) {
        return false;
    }
    let response: &[u8] = if pool.is_draining() && buffer[..bytes_read].starts_with(b"GET /readyz ") {
        b"HTTP/1.1 503 Service Unavailable\r\ncontent-type: text/plain\r\ncontent-length: 9\r\nconnection: close\r\n\r\ndraining\n"
    } else {
        b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 3\r\nconnection: close\r\n\r\nok\n"
    };
    let _ = stream.write_all(response).await;
    true
}

fn is_health_probe(buffer: &[u8]) -> bool {
    buffer.starts_with(b"GET /readyz HTTP/1.1\r\n")
        || buffer.starts_with(b"GET /readyz HTTP/1.0\r\n")
        || buffer.starts_with(b"GET /healthz HTTP/1.1\r\n")
        || buffer.starts_with(b"GET /healthz HTTP/1.0\r\n")
}

/// Requests that would execute against host-local paths in hosted mode.
fn denies_local_workspace(request: &JsonRpcRequest) -> bool {
    match request.method.as_str() {
        "workspace/create" => true,
        "thread/start" => request
            .params
            .as_ref()
            .and_then(|params| params.get("cwd"))
            .is_some_and(|cwd| !cwd.is_null()),
        _ => false,
    }
}

fn send_error(
    outbound: &tokio::sync::mpsc::UnboundedSender<OutboundMessage>,
    id: serde_json::Value,
    code: i32,
    message: &str,
) {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    };
    if let Ok(text) = serde_json::to_string(&response) {
        let _ = outbound.send(OutboundMessage::Control(Message::Text(text.into())));
    }
}

/// Lifecycle methods that fan out to tenant hooks.
fn hook_event_kind(method: &str) -> Option<&'static str> {
    match method {
        "thread/start" | "thread/fork" => Some("thread.started"),
        "turn/start" => Some("turn.started"),
        _ => None,
    }
}

/// `hosted/*` administration handled in the gateway.
fn handle_hosted_method(
    context: &HostedRequestContext,
    authenticator: &HostedAuthenticator,
    tenants: &TenantRegistry,
    audit: &AuditLog,
    hooks: &HookStore,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let ok = |result: serde_json::Value| JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id.clone()),
        result: Some(result),
        error: None,
    };
    match request.method.as_str() {
        "hosted/whoami" => ok(serde_json::json!({
            "tenant": context.tenant,
            "principal": context.principal,
            "role": context.role,
            "scopes": context.scopes,
        })),
        "hosted/tenants/list" => ok(serde_json::json!({ "tenants": tenants.list() })),
        "hosted/service_accounts/create" => {
            let display_name = request
                .params
                .as_ref()
                .and_then(|params| params.get("displayName"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("service-account")
                .to_string();
            let key = authenticator.mint_service_account_key(
                PrincipalSeed {
                    tenant_id: context.tenant.tenant_id.clone(),
                    principal: roder_api::identity::PrincipalContext::ServiceAccount {
                        service_account_id: format!("sa-{}", uuid::Uuid::new_v4().simple()),
                        display_name: Some(display_name),
                    },
                    role: HostedRole::Member,
                    scopes: vec![HostedScope::Read, HostedScope::Write],
                },
                None,
            );
            audit.record(AuditRecord {
                kind: "service_account_created".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: Some(format!("sa:{}", key.key_id)),
                method: Some(request.method.clone()),
                reason: None,
                timestamp: OffsetDateTime::now_utc(),
            });
            // The token is returned exactly once; only its hash is stored.
            ok(serde_json::json!({ "keyId": key.key_id, "token": key.token }))
        }
        "hosted/service_accounts/revoke" => {
            let key_id = request
                .params
                .as_ref()
                .and_then(|params| params.get("keyId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let revoked = authenticator.revoke_service_account_key(&key_id);
            if revoked {
                audit.record(AuditRecord {
                    kind: "service_account_revoked".to_string(),
                    tenant_id: Some(context.tenant.tenant_id.clone()),
                    principal_id: Some(context.principal.id().to_string()),
                    credential_id: Some(format!("sa:{key_id}")),
                    method: Some(request.method.clone()),
                    reason: None,
                    timestamp: OffsetDateTime::now_utc(),
                });
            }
            ok(serde_json::json!({ "revoked": revoked }))
        }
        "hosted/audit/list" => {
            let records = audit.for_tenant(&context.tenant.tenant_id);
            ok(serde_json::json!({ "records": records }))
        }
        "hosted/hooks/create" => {
            let Some(definition) = request
                .params
                .as_ref()
                .and_then(|params| params.get("hook"))
                .cloned()
                .and_then(|hook| serde_json::from_value(hook).ok())
            else {
                return error_response(id, -32602, "params.hook must be a hook definition");
            };
            match hooks.create(&context.tenant.tenant_id, definition) {
                Ok(created) => {
                    audit.record(AuditRecord {
                        kind: "hook_change".to_string(),
                        tenant_id: Some(context.tenant.tenant_id.clone()),
                        principal_id: Some(context.principal.id().to_string()),
                        credential_id: context.credential_id.clone(),
                        method: Some(request.method.clone()),
                        reason: Some(format!("created {}", created.id)),
                        timestamp: OffsetDateTime::now_utc(),
                    });
                    ok(serde_json::json!({ "hook": created }))
                }
                Err(error) => error_response(id, -32602, &error.to_string()),
            }
        }
        "hosted/hooks/list" => ok(serde_json::json!({
            "hooks": hooks.list(&context.tenant.tenant_id),
        })),
        "hosted/hooks/delete" => {
            let hook_id = request
                .params
                .as_ref()
                .and_then(|params| params.get("hookId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let deleted = hooks.delete(&context.tenant.tenant_id, &hook_id);
            if deleted {
                audit.record(AuditRecord {
                    kind: "hook_change".to_string(),
                    tenant_id: Some(context.tenant.tenant_id.clone()),
                    principal_id: Some(context.principal.id().to_string()),
                    credential_id: context.credential_id.clone(),
                    method: Some(request.method.clone()),
                    reason: Some(format!("deleted {hook_id}")),
                    timestamp: OffsetDateTime::now_utc(),
                });
            }
            ok(serde_json::json!({ "deleted": deleted }))
        }
        other => error_response(
            id,
            -32601,
            &format!("hosted method {other} is not implemented yet"),
        ),
    }
}

fn error_response(id: serde_json::Value, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    }
}
