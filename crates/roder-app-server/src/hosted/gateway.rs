//! Hosted multi-tenant WebSocket gateway.
//!
//! Every connection authenticates with a bearer credential in the
//! `Authorization` header at handshake time (query-string credentials are
//! always rejected) and resolves to a `HostedRequestContext` before any
//! JSON-RPC dispatch. Per request the gateway enforces, in order: frame
//! size, rate limit, method authorization, and thread-ownership isolation;
//! only then does the wrapped app-server see the request. `hosted/*`
//! administration is handled in the gateway itself — the app-server stays
//! tenant-unaware until phase 72 Task 3 introduces per-tenant runtimes.
//!
//! Thread isolation model (pre-Task 3): the gateway records which tenant
//! created each thread (via this gateway) and rejects requests referencing
//! threads owned by other tenants; notifications carrying a `threadId`
//! route only to the owning tenant's connections, and notifications
//! without one are not forwarded to hosted clients.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

use futures::{SinkExt, StreamExt};
use roder_api::identity::{AuthorizationDecision, HostedRequestContext, HostedRole, HostedScope};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::server::AppServer;

use super::audit::{AuditLog, AuditRecord};
use super::auth::{HostedAuthenticator, PrincipalSeed};
use super::authorization::authorize_method;
use super::rate_limit::{RateLimitConfig, RateLimiter};
use super::tenant::TenantRegistry;

pub struct HostedGatewayOptions {
    pub listen: String,
    pub authenticator: Arc<HostedAuthenticator>,
    pub tenants: Arc<TenantRegistry>,
    pub audit: Arc<AuditLog>,
    pub limits: RateLimitConfig,
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

/// Shared gateway state: tenant ownership of threads created through it.
#[derive(Default)]
struct ThreadOwners {
    owners: RwLock<HashMap<String, String>>,
}

impl ThreadOwners {
    fn owner(&self, thread_id: &str) -> Option<String> {
        self.owners.read().unwrap().get(thread_id).cloned()
    }

    fn claim(&self, thread_id: &str, tenant_id: &str) {
        self.owners
            .write()
            .unwrap()
            .insert(thread_id.to_string(), tenant_id.to_string());
    }
}

pub async fn serve_hosted_gateway(
    app_server: Arc<AppServer>,
    options: HostedGatewayOptions,
) -> anyhow::Result<HostedGatewayController> {
    let listener = TcpListener::bind(&options.listen).await?;
    let listen_addr = listener.local_addr()?;
    let limiter = Arc::new(RateLimiter::new(options.limits));
    let owners = Arc::new(ThreadOwners::default());
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
            let app_server = app_server.clone();
            let authenticator = options.authenticator.clone();
            let tenants = options.tenants.clone();
            let audit = options.audit.clone();
            let limiter = limiter.clone();
            let owners = owners.clone();
            connections.spawn(async move {
                serve_connection(app_server, authenticator, tenants, audit, limiter, owners, stream)
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

async fn serve_connection(
    app_server: Arc<AppServer>,
    authenticator: Arc<HostedAuthenticator>,
    tenants: Arc<TenantRegistry>,
    audit: Arc<AuditLog>,
    limiter: Arc<RateLimiter>,
    owners: Arc<ThreadOwners>,
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
            match callback_authenticator.authenticate(token, &callback_tenants, OffsetDateTime::now_utc()) {
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

    // Notifications: only events for threads owned by this tenant, and only
    // with the events (read) scope.
    if context.has_scope(HostedScope::Read) {
        let mut notifications = app_server.subscribe_notifications();
        let notification_tx = outbound_tx.clone();
        let notification_owners = owners.clone();
        let tenant_id = context.tenant.tenant_id.clone();
        subtasks.spawn(async move {
            while let Ok(notification) = notifications.recv().await {
                let thread_id = notification
                    .params
                    .get("threadId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let owned = thread_id
                    .as_deref()
                    .and_then(|thread_id| notification_owners.owner(thread_id))
                    .is_some_and(|owner| owner == tenant_id);
                if !owned {
                    continue;
                }
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

        // Thread-ownership isolation for requests that reference a thread.
        if let Some(thread_id) = request
            .params
            .as_ref()
            .and_then(|params| params.get("threadId"))
            .and_then(serde_json::Value::as_str)
            && owners
                .owner(thread_id)
                .is_some_and(|owner| owner != context.tenant.tenant_id)
        {
            audit.record(AuditRecord {
                kind: "method_denied".to_string(),
                tenant_id: Some(context.tenant.tenant_id.clone()),
                principal_id: Some(context.principal.id().to_string()),
                credential_id: context.credential_id.clone(),
                method: Some(request.method.clone()),
                reason: Some("wrong_tenant".to_string()),
                timestamp: OffsetDateTime::now_utc(),
            });
            send_error(&outbound_tx, id, -32012, "forbidden: wrong_tenant");
            continue;
        }

        let response = if request.method.starts_with("hosted/") {
            handle_hosted_method(&context, &authenticator, &tenants, &audit, request)
        } else {
            let method = request.method.clone();
            let response = app_server.handle_request(request).await;
            // Track tenant ownership of threads created through this
            // gateway connection.
            if matches!(method.as_str(), "thread/start" | "thread/fork")
                && let Some(result) = &response.result
            {
                let created = result
                    .get("thread")
                    .and_then(|thread| thread.get("id"))
                    .or_else(|| result.get("id"))
                    .and_then(serde_json::Value::as_str);
                if let Some(thread_id) = created {
                    owners.claim(thread_id, &context.tenant.tenant_id);
                }
            }
            response
        };
        if let Ok(text) = serde_json::to_string(&response) {
            let _ = outbound_tx.send(Message::Text(text.into()));
        }
    }
    subtasks.abort_all();
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

/// `hosted/*` administration handled in the gateway.
fn handle_hosted_method(
    context: &HostedRequestContext,
    authenticator: &HostedAuthenticator,
    tenants: &TenantRegistry,
    audit: &AuditLog,
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
        other => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("hosted method {other} is not implemented yet"),
                data: None,
            }),
        },
    }
}
