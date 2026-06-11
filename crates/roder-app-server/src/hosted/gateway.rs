//! Hosted multi-tenant WebSocket gateway.
//!
//! Every connection authenticates with a bearer credential in the
//! `Authorization` header at handshake time (query-string credentials are
//! always rejected) and resolves to a `HostedRequestContext` before any
//! JSON-RPC dispatch. Per request the gateway enforces, in order: frame
//! size, rate limit, method authorization, and hosted workspace policy;
//! only then does the tenant's app-server see the request.
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
use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use roder_api::identity::{AuthorizationDecision, HostedRequestContext, HostedRole, HostedScope};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use super::audit::{AuditLog, AuditRecord};
use super::auth::{HostedAuthenticator, PrincipalSeed};
use super::authorization::authorize_method;
use super::hook_delivery::HookDeliveryService;
use super::hooks::HookStore;
use super::rate_limit::{RateLimitConfig, RateLimiter};
use super::runtime_pool::HostedRuntimePool;
use super::tenant::TenantRegistry;

pub struct HostedGatewayOptions {
    pub listen: String,
    pub authenticator: Arc<HostedAuthenticator>,
    pub tenants: Arc<TenantRegistry>,
    pub audit: Arc<AuditLog>,
    pub limits: RateLimitConfig,
    pub hooks: Arc<HookStore>,
    pub hook_delivery: Arc<HookDeliveryService>,
}

pub struct HostedGatewayController {
    pub listen_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl HostedGatewayController {
    pub async fn stop(mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await?;
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
        loop {
            let accepted = tokio::select! {
                _ = &mut shutdown_rx => break,
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
            connections.spawn(async move {
                serve_connection(
                    pool,
                    authenticator,
                    tenants,
                    audit,
                    limiter,
                    hooks,
                    hook_delivery,
                    stream,
                )
                .await;
            });
        }
        connections.abort_all();
    });

    Ok(HostedGatewayController {
        listen_addr,
        shutdown: Some(shutdown_tx),
        task,
    })
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
    stream: tokio::net::TcpStream,
) {
    // Authenticate at handshake time, before any request dispatch.
    let context: Arc<Mutex<Option<HostedRequestContext>>> = Arc::default();
    let callback_context = context.clone();
    let callback_audit = audit.clone();
    let callback_authenticator = authenticator.clone();
    let callback_tenants = tenants.clone();
    #[allow(clippy::result_large_err)]
    let callback =
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
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
            let bearer = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "));
            let Some(token) = bearer else {
                return Err(deny("missing_credential"));
            };
            match callback_authenticator.authenticate(
                token,
                &callback_tenants,
                OffsetDateTime::now_utc(),
            ) {
                Ok(resolved) => {
                    callback_audit.record(AuditRecord {
                        kind: "auth_ok".to_string(),
                        tenant_id: Some(resolved.tenant.tenant_id.clone()),
                        principal_id: Some(resolved.principal.id().to_string()),
                        credential_id: resolved.credential_id.clone(),
                        method: None,
                        reason: None,
                        timestamp: OffsetDateTime::now_utc(),
                    });
                    *callback_context.lock().unwrap() = Some(resolved);
                    Ok(response)
                }
                Err(error) => Err(deny(&error.to_string().replace(' ', "_"))),
            }
        };

    let Ok(websocket) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let Some(context) = context.lock().unwrap().clone() else {
        return;
    };

    // Resolve the tenant's runtime and hold the lease for the lifetime of
    // the connection so it is never idle-evicted mid-session.
    let lease = match pool.lease(&context.tenant.tenant_id).await {
        Ok(lease) => lease,
        Err(error) => {
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

    let (mut ws_write, mut ws_read) = websocket.split();
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let mut subtasks = tokio::task::JoinSet::new();
    subtasks.spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            if ws_write.send(message).await.is_err() {
                break;
            }
        }
    });

    // Notifications come from the tenant's own app-server, so everything on
    // this stream already belongs to this tenant; the read scope gates the
    // subscription itself.
    if context.has_scope(HostedScope::Read) {
        let mut notifications = app_server.subscribe_notifications();
        let notification_tx = outbound_tx.clone();
        subtasks.spawn(async move {
            while let Ok(notification) = notifications.recv().await {
                let Ok(text) = serde_json::to_string(&notification) else {
                    continue;
                };
                if notification_tx.send(Message::Text(text.into())).is_err() {
                    break;
                }
            }
        });
    }

    while let Some(Ok(message)) = ws_read.next().await {
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        if text.len() > limiter.max_request_bytes() {
            send_error(&outbound_tx, serde_json::Value::Null, -32600, "request too large");
            continue;
        }
        let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&text) else {
            send_error(&outbound_tx, serde_json::Value::Null, -32700, "parse error");
            continue;
        };
        let id = request.id.clone().unwrap_or(serde_json::Value::Null);

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

        let response = if request.method.starts_with("hosted/") {
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
            let _ = outbound_tx.send(Message::Text(text.into()));
        }
    }
    subtasks.abort_all();
    drop(lease);
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
    outbound: &tokio::sync::mpsc::UnboundedSender<Message>,
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
        let _ = outbound.send(Message::Text(text.into()));
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
        other => error_response(id, -32601, &format!("hosted method {other} is not implemented yet")),
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
