//! Hosted gateway e2e over loopback WebSocket (phase 72, Tasks 2–3): auth
//! before dispatch, query-token rejection, hosted/whoami + service-account
//! lifecycle, per-tenant runtime isolation (stores and notifications),
//! hosted workspace policy, idle eviction, and rate limits. Fully offline
//! with the fake inference engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use futures::{SinkExt, StreamExt};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::identity::{
    HostedRequestContext, HostedRole, HostedScope, PrincipalContext, TenantContext,
};
use roder_app_server::AppServer;
use roder_app_server::hosted::auth::PrincipalSeed;
use roder_app_server::hosted::{
    AllowAllHostedRequestPolicy, AuditLog, ExternalBearerVerifier, HostedAuthError,
    HostedAuthenticator, HostedGatewayOptions, HostedRequestPolicy, HostedRequestPolicyDecision,
    HostedRuntimePool, HostedRuntimeProfile, RateLimitConfig, TenantRegistry, serve_hosted_gateway,
};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_protocol::{JsonRpcRequest, JsonRpcResponse};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-hosted-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Per-tenant app-server factory: every tenant gets its own runtime with a
/// private JSONL thread store under its data directory.
fn tenant_pool(label: &str, allow_local_workspaces: bool) -> Arc<HostedRuntimePool> {
    tenant_pool_with_ttl(
        label,
        allow_local_workspaces,
        std::time::Duration::from_secs(3600),
    )
}

fn tenant_pool_with_ttl(
    label: &str,
    allow_local_workspaces: bool,
    idle_ttl: std::time::Duration,
) -> Arc<HostedRuntimePool> {
    Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir(label),
            allow_local_workspaces,
            idle_ttl,
        },
        Arc::new(|_tenant_id, data_dir| {
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
                    base_path: data_dir.join("threads"),
                }));
                let runtime = Arc::new(Runtime::new(
                    builder.build().unwrap(),
                    RuntimeConfig::default(),
                )?);
                Ok(Arc::new(AppServer::new(runtime)))
            })
        }),
    ))
}

struct Fixture {
    controller: roder_app_server::hosted::HostedGatewayController,
    authenticator: Arc<HostedAuthenticator>,
    audit: Arc<AuditLog>,
    url: String,
}

async fn fixture(label: &str, limits: RateLimitConfig, allow_local_workspaces: bool) -> Fixture {
    fixture_with_policy(
        label,
        limits,
        allow_local_workspaces,
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await
}

async fn fixture_with_policy(
    label: &str,
    limits: RateLimitConfig,
    allow_local_workspaces: bool,
    request_policy: Arc<dyn HostedRequestPolicy>,
) -> Fixture {
    let authenticator = Arc::new(HostedAuthenticator::default());
    let tenants = Arc::new(TenantRegistry::default());
    let audit = Arc::new(AuditLog::default());
    for tenant in ["tenant-a", "tenant-b"] {
        tenants.insert(TenantContext {
            tenant_id: tenant.to_string(),
            display_name: None,
        });
        authenticator
            .register_static_key(
                &format!("rk_test_{}_writer", tenant.replace('-', "_")),
                PrincipalSeed {
                    tenant_id: tenant.to_string(),
                    principal: PrincipalContext::User {
                        user_id: format!("user-{tenant}"),
                        display_name: None,
                    },
                    role: HostedRole::TenantAdmin,
                    scopes: vec![HostedScope::Read, HostedScope::Write, HostedScope::Admin],
                },
            )
            .unwrap();
    }
    let controller = serve_hosted_gateway(
        tenant_pool(label, allow_local_workspaces),
        HostedGatewayOptions {
            listen: "127.0.0.1:0".to_string(),
            authenticator: authenticator.clone(),
            tenants,
            audit: audit.clone(),
            limits,
            hooks: Arc::new(roder_app_server::hosted::HookStore::default()),
            hook_delivery: Arc::new(roder_app_server::hosted::HookDeliveryService::new(
                Default::default(),
            )),
            request_policy,
        },
    )
    .await
    .unwrap();
    let url = format!("ws://{}", controller.listen_addr);
    Fixture {
        controller,
        authenticator,
        audit,
        url,
    }
}

async fn connect(url: &str, token: &str) -> anyhow::Result<Socket> {
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    let (socket, _) = tokio_tungstenite::connect_async(request).await?;
    Ok(socket)
}

async fn call(socket: &mut Socket, method: &str, params: serde_json::Value) -> JsonRpcResponse {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params: Some(params),
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&request).unwrap().into(),
        ))
        .await
        .unwrap();
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .expect("response timeout")
            .expect("socket closed")
            .unwrap();
        if let Message::Text(text) = message {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            // Skip interleaved notifications.
            if value.get("id").is_some_and(|id| !id.is_null()) {
                return serde_json::from_value(value).unwrap();
            }
        }
    }
}

struct RewriteAndDenyPolicy;

impl HostedRequestPolicy for RewriteAndDenyPolicy {
    fn evaluate(
        &self,
        context: &roder_api::identity::HostedRequestContext,
        bearer_token: &str,
        mut request: JsonRpcRequest,
    ) -> HostedRequestPolicyDecision {
        assert_eq!(context.tenant.tenant_id, "tenant-a");
        assert_eq!(bearer_token, "rk_test_tenant_a_writer");
        match request.method.as_str() {
            "client/whoami" => {
                request.method = "hosted/whoami".to_string();
                HostedRequestPolicyDecision::allow(request)
            }
            "thread/list" => HostedRequestPolicyDecision::deny(format!(
                "policy_blocked credential={bearer_token}"
            )),
            _ => HostedRequestPolicyDecision::allow(request),
        }
    }
}

struct ExpiringExternalVerifier {
    valid: AtomicBool,
    checks: AtomicUsize,
}

impl ExpiringExternalVerifier {
    fn new() -> Self {
        Self {
            valid: AtomicBool::new(true),
            checks: AtomicUsize::new(0),
        }
    }
}

impl ExternalBearerVerifier for ExpiringExternalVerifier {
    fn verify_bearer(
        &self,
        token: &str,
        now: time::OffsetDateTime,
    ) -> Result<Option<HostedRequestContext>, HostedAuthError> {
        if token != "external-expiring-token" {
            return Ok(None);
        }
        self.checks.fetch_add(1, Ordering::SeqCst);
        if !self.valid.load(Ordering::SeqCst) {
            return Err(HostedAuthError::Expired);
        }
        Ok(Some(HostedRequestContext {
            tenant: TenantContext {
                tenant_id: "external-tenant".to_string(),
                display_name: None,
            },
            principal: PrincipalContext::User {
                user_id: "external-user".to_string(),
                display_name: None,
            },
            role: HostedRole::Member,
            scopes: vec![HostedScope::Read, HostedScope::Write],
            credential_id: Some("external:session-1".to_string()),
            authenticated_at: now,
        }))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_authenticates_and_serves_whoami_and_service_accounts() {
    let fixture = fixture("whoami", RateLimitConfig::default(), true).await;

    // Bad token: handshake fails before any dispatch.
    assert!(connect(&fixture.url, "rk_test_wrong").await.is_err());
    // Query-string credentials are rejected outright.
    let query_url = format!("{}/?token=rk_test_tenant_a_writer", fixture.url);
    let request = query_url.into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());

    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let response = call(&mut socket, "initialize", serde_json::json!({})).await;
    assert!(response.error.is_none());

    let whoami = call(&mut socket, "hosted/whoami", serde_json::json!({})).await;
    let result = whoami.result.unwrap();
    assert_eq!(result["tenant"]["tenantId"], "tenant-a");
    assert_eq!(result["principal"]["user_id"], "user-tenant-a");

    // Service-account lifecycle: mint, use, revoke.
    let minted = call(
        &mut socket,
        "hosted/service_accounts/create",
        serde_json::json!({ "displayName": "ci" }),
    )
    .await;
    let token = minted.result.as_ref().unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let key_id = minted.result.as_ref().unwrap()["keyId"]
        .as_str()
        .unwrap()
        .to_string();

    let mut sa_socket = connect(&fixture.url, &token).await.unwrap();
    let sa_whoami = call(&mut sa_socket, "hosted/whoami", serde_json::json!({})).await;
    assert_eq!(sa_whoami.result.unwrap()["tenant"]["tenantId"], "tenant-a");

    let revoked = call(
        &mut socket,
        "hosted/service_accounts/revoke",
        serde_json::json!({ "keyId": key_id }),
    )
    .await;
    assert_eq!(revoked.result.unwrap()["revoked"], true);

    // Revocation applies to the already-open service-account socket before
    // another request can reach the tenant runtime.
    let denied_existing = call(&mut sa_socket, "hosted/whoami", serde_json::json!({})).await;
    let error = denied_existing
        .error
        .expect("revoked socket must be denied");
    assert_eq!(error.code, -32013);
    assert!(error.message.contains("credential_revoked"));
    assert!(connect(&fixture.url, &token).await.is_err());

    // Audit shows the lifecycle without leaking secrets.
    let audit = call(&mut socket, "hosted/audit/list", serde_json::json!({})).await;
    let records = serde_json::to_string(&audit.result.unwrap()).unwrap();
    assert!(records.contains("service_account_created"));
    assert!(records.contains("service_account_revoked"));
    assert!(
        !records.contains(&token),
        "audit must never contain raw tokens"
    );

    let _ = fixture.authenticator;
    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_external_bearers_are_revalidated_and_closed_without_notification_leaks() {
    let authenticator = Arc::new(HostedAuthenticator::default());
    let verifier = Arc::new(ExpiringExternalVerifier::new());
    authenticator.register_external_bearer_verifier(verifier.clone());
    let audit = Arc::new(AuditLog::default());
    let controller = serve_hosted_gateway(
        tenant_pool("external-revalidation", true),
        HostedGatewayOptions {
            listen: "127.0.0.1:0".to_string(),
            authenticator,
            tenants: Arc::new(TenantRegistry::default()),
            audit: audit.clone(),
            limits: RateLimitConfig::default(),
            hooks: Arc::new(roder_app_server::hosted::HookStore::default()),
            hook_delivery: Arc::new(roder_app_server::hosted::HookDeliveryService::new(
                Default::default(),
            )),
            request_policy: Arc::new(AllowAllHostedRequestPolicy),
        },
    )
    .await
    .unwrap();
    let url = format!("ws://{}", controller.listen_addr);

    let mut socket = connect(&url, "external-expiring-token").await.unwrap();
    assert_eq!(verifier.checks.load(Ordering::SeqCst), 1);
    assert!(
        call(&mut socket, "initialize", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    assert_eq!(verifier.checks.load(Ordering::SeqCst), 2);

    verifier.valid.store(false, Ordering::SeqCst);
    // Send nothing else. The gateway's independent auth timer must terminate
    // this passive socket, and no tenant notification may drain after the
    // credential has been rejected.
    let saw_terminal_auth_error = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut saw_terminal_auth_error = false;
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    assert!(!text.contains("external-expiring-token"));
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert!(
                        value.get("method").is_none(),
                        "notifications must stop once auth is invalid: {value}"
                    );
                    let response: JsonRpcResponse = serde_json::from_value(value).unwrap();
                    let error = response
                        .error
                        .expect("terminal frame must be an auth error");
                    assert_eq!(error.code, -32013);
                    assert!(error.message.contains("credential_expired"));
                    saw_terminal_auth_error = true;
                }
                Some(Ok(Message::Close(_))) | None => return saw_terminal_auth_error,
                Some(Ok(_)) => {}
                Some(Err(error)) => panic!("unexpected socket error before close: {error}"),
            }
        }
    })
    .await
    .expect("idle expired socket was not closed by the auth timer");
    assert!(saw_terminal_auth_error);
    assert!(verifier.checks.load(Ordering::SeqCst) >= 3);

    let records = serde_json::to_string(&audit.for_tenant("external-tenant")).unwrap();
    assert!(records.contains("auth_revalidation_failed"));
    assert!(records.contains("credential_expired"));
    assert!(!records.contains("external-expiring-token"));

    controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_subprotocol_auth_requires_and_echoes_remote_protocol() {
    let fixture = fixture("browser-auth", RateLimitConfig::default(), true).await;
    let mut request = fixture.url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "roder.remote.v1, bearer.rk_test_tenant_a_writer"
            .parse()
            .unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok()),
        Some("roder.remote.v1")
    );
    assert!(
        call(&mut socket, "initialize", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    socket.close(None).await.unwrap();

    let mut missing_protocol = fixture.url.as_str().into_client_request().unwrap();
    missing_protocol.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "bearer.rk_test_tenant_a_writer".parse().unwrap(),
    );
    assert!(
        tokio_tungstenite::connect_async(missing_protocol)
            .await
            .is_err()
    );

    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hosted_health_endpoints_do_not_require_auth() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let fixture = fixture("health", RateLimitConfig::default(), true).await;
    for path in ["/readyz", "/healthz"] {
        let mut stream = tokio::net::TcpStream::connect(fixture.controller.listen_addr)
            .await
            .unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: roder\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buffer = [0_u8; 512];
        let bytes_read = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..bytes_read]);

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.ends_with("\r\n\r\nok\n"));
    }

    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn request_policy_can_rewrite_or_deny_without_leaking_bearer() {
    let fixture = fixture_with_policy(
        "request-policy",
        RateLimitConfig::default(),
        true,
        Arc::new(RewriteAndDenyPolicy),
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut socket, "initialize", serde_json::json!({}))
            .await
            .error
            .is_none()
    );

    let rewritten = call(&mut socket, "client/whoami", serde_json::json!({})).await;
    assert_eq!(rewritten.result.unwrap()["tenant"]["tenantId"], "tenant-a");

    let denied = call(&mut socket, "thread/list", serde_json::json!({})).await;
    let error = denied.error.unwrap();
    assert_eq!(error.code, -32012);
    assert!(error.message.contains("policy_blocked"));
    assert!(!error.message.contains("rk_test_tenant_a_writer"));

    let audit = serde_json::to_string(&fixture.audit.for_tenant("tenant-a")).unwrap();
    assert!(audit.contains("request_policy_denied"));
    assert!(!audit.contains("rk_test_tenant_a_writer"));

    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tenants_run_isolated_runtimes_with_isolated_stores_and_notifications() {
    let fixture = fixture("isolation", RateLimitConfig::default(), true).await;
    let mut tenant_a = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let mut tenant_b = connect(&fixture.url, "rk_test_tenant_b_writer")
        .await
        .unwrap();
    for socket in [&mut tenant_a, &mut tenant_b] {
        assert!(
            call(socket, "initialize", serde_json::json!({}))
                .await
                .error
                .is_none()
        );
    }

    let workspace = temp_dir("isolation-ws");
    let created = call(
        &mut tenant_a,
        "workspace/create",
        serde_json::json!({
            "name": null,
            "roots": [{ "path": workspace.display().to_string(), "name": null }],
            "defaultRootPath": workspace.display().to_string(),
        }),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let workspace_result = created.result.unwrap();
    let started = call(
        &mut tenant_a,
        "thread/start",
        serde_json::json!({
            "workspaceId": workspace_result["workspace"]["id"],
            "rootId": workspace_result["workspace"]["defaultRootId"],
            "model": "mock",
            "modelProvider": null,
            "reasoning": null,
        }),
    )
    .await;
    assert!(started.error.is_none(), "{:?}", started.error);
    let thread_id = started.result.unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Tenant B's runtime has never seen the thread: list is empty and reads
    // fail — isolation by construction, not by request filtering.
    let listed = call(&mut tenant_b, "thread/list", serde_json::json!({})).await;
    let count = listed.result.unwrap()["data"].as_array().map(Vec::len);
    assert_eq!(count, Some(0), "tenant B must not list tenant A threads");
    // thread/read returns no data for B (its store has no such thread) and
    // archive fails outright.
    let read = call(
        &mut tenant_b,
        "thread/read",
        serde_json::json!({ "threadId": thread_id }),
    )
    .await;
    let leaked = read
        .result
        .as_ref()
        .and_then(|result| result.get("thread"))
        .is_some_and(|thread| !thread.is_null());
    assert!(
        read.error.is_some() || !leaked,
        "thread/read must not leak tenant A data: {:?}",
        read.result
    );
    // Archive on B's runtime is a no-op against B's empty store; tenant A's
    // thread stays fully readable afterwards.
    let _ = call(
        &mut tenant_b,
        "thread/archive",
        serde_json::json!({ "threadId": thread_id }),
    )
    .await;
    let still_there = call(
        &mut tenant_a,
        "thread/read",
        serde_json::json!({ "threadId": thread_id }),
    )
    .await;
    let thread = &still_there.result.expect("tenant A read")["thread"];
    assert_eq!(thread["id"].as_str(), Some(thread_id.as_str()));

    // A turn in tenant A's thread emits notifications to A but never to B.
    let turn = call(
        &mut tenant_a,
        "turn/start",
        serde_json::json!({
            "threadId": thread_id,
            "prompt": "hello",
            "mcpAuthToken": "turn-scoped-mcp-token"
        }),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);
    assert_eq!(
        roder_api::mcp_auth::thread_token(&thread_id).as_deref(),
        Some("turn-scoped-mcp-token")
    );

    let mut a_saw_notification = false;
    for _ in 0..50 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), tenant_a.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value.get("method").is_some()
                    && value["params"]["threadId"].as_str() == Some(thread_id.as_str())
                {
                    a_saw_notification = true;
                    break;
                }
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(
        a_saw_notification,
        "owner tenant must receive thread notifications"
    );

    // Tenant B's socket stays silent (its runtime emitted nothing).
    let quiet = tokio::time::timeout(std::time::Duration::from_millis(500), tenant_b.next()).await;
    assert!(
        quiet.is_err(),
        "tenant B must not receive tenant A notifications"
    );

    roder_api::mcp_auth::clear_thread_token(&thread_id);
    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_dispatches_tenant_hooks_on_thread_start() {
    // Minimal always-200 hook target counting hits.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_url = format!("http://{}/hook", listener.local_addr().unwrap());
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let task_hits = hits.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer).await;
            task_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await;
        }
    });

    let fixture = fixture("hooks-e2e", RateLimitConfig::default(), true).await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut socket, "initialize", serde_json::json!({}))
            .await
            .error
            .is_none()
    );

    let created = call(
        &mut socket,
        "hosted/hooks/create",
        serde_json::json!({ "hook": {
            "id": "hook-e2e",
            "scope": "tenant",
            "eventKinds": ["thread."],
            "url": hook_url,
            "enabled": true,
            "createdAt": "1970-01-01T00:00:00Z",
            "updatedAt": "1970-01-01T00:00:00Z",
        }}),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let listed = call(&mut socket, "hosted/hooks/list", serde_json::json!({})).await;
    assert_eq!(
        listed.result.unwrap()["hooks"].as_array().map(Vec::len),
        Some(1)
    );

    let workspace = temp_dir("hooks-ws");
    let created = call(
        &mut socket,
        "workspace/create",
        serde_json::json!({
            "name": null,
            "roots": [{ "path": workspace.display().to_string(), "name": null }],
            "defaultRootPath": workspace.display().to_string(),
        }),
    )
    .await;
    let workspace_result = created.result.unwrap();
    let started = call(
        &mut socket,
        "thread/start",
        serde_json::json!({
            "workspaceId": workspace_result["workspace"]["id"],
            "rootId": workspace_result["workspace"]["defaultRootId"],
            "model": "mock",
            "modelProvider": null,
            "reasoning": null,
        }),
    )
    .await;
    assert!(started.error.is_none(), "{:?}", started.error);

    // The hook fires asynchronously; poll briefly.
    let mut delivered = false;
    for _ in 0..40 {
        if hits.load(std::sync::atomic::Ordering::SeqCst) >= 1 {
            delivered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(delivered, "thread/start must dispatch the tenant hook");

    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hosted_profile_denies_local_workspaces_by_default() {
    let fixture = fixture("no-local-ws", RateLimitConfig::default(), false).await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();

    let denied = call(
        &mut socket,
        "workspace/create",
        serde_json::json!({
            "name": null,
            "roots": [{ "path": "/etc", "name": null }],
            "defaultRootPath": "/etc",
        }),
    )
    .await;
    let error = denied.error.expect("workspace/create must be denied");
    assert!(
        error.message.contains("local_workspace_disabled"),
        "{}",
        error.message
    );

    let denied = call(
        &mut socket,
        "thread/start",
        serde_json::json!({ "workspaceId": "w", "cwd": "/etc" }),
    )
    .await;
    let error = denied.error.expect("thread/start with cwd must be denied");
    assert!(
        error.message.contains("local_workspace_disabled"),
        "{}",
        error.message
    );

    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_tenant_runtimes_evict_without_touching_busy_ones() {
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("evict"),
            allow_local_workspaces: true,
            idle_ttl: std::time::Duration::from_millis(0),
        },
        Arc::new(|_tenant_id, data_dir| {
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
                    base_path: data_dir.join("threads"),
                }));
                let runtime = Arc::new(Runtime::new(
                    builder.build().unwrap(),
                    RuntimeConfig::default(),
                )?);
                Ok(Arc::new(AppServer::new(runtime)))
            })
        }),
    ));

    // A held lease (in-flight request / resident connection) blocks
    // eviction; a released one does not.
    let lease_a = pool.lease("tenant-a").await.unwrap();
    let lease_b = pool.lease("tenant-b").await.unwrap();
    drop(lease_b);
    assert_eq!(pool.len().await, 2);

    let evicted = pool.evict_idle().await;
    assert_eq!(evicted, vec!["tenant-b".to_string()]);
    assert_eq!(pool.len().await, 1);

    drop(lease_a);
    let evicted = pool.evict_idle().await;
    assert_eq!(evicted, vec!["tenant-a".to_string()]);
    assert!(pool.is_empty().await);

    // Per-tenant data directories are distinct (separate stores and
    // automation databases by construction).
    let lease_a = pool.lease("tenant-a").await.unwrap();
    let lease_b = pool.lease("tenant-b").await.unwrap();
    assert!(!Arc::ptr_eq(&lease_a.server, &lease_b.server));
    pool.shutdown(std::time::Duration::from_secs(1)).await;
    assert!(pool.is_empty().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_periodically_evicts_idle_runtimes_and_stops_on_shutdown() {
    let pool = tenant_pool_with_ttl(
        "periodic-eviction",
        false,
        std::time::Duration::from_millis(20),
    );
    let controller = serve_hosted_gateway(
        pool.clone(),
        HostedGatewayOptions {
            listen: "127.0.0.1:0".to_string(),
            authenticator: Arc::new(HostedAuthenticator::default()),
            tenants: Arc::new(TenantRegistry::default()),
            audit: Arc::new(AuditLog::default()),
            limits: RateLimitConfig::default(),
            hooks: Arc::new(roder_app_server::hosted::HookStore::default()),
            hook_delivery: Arc::new(roder_app_server::hosted::HookDeliveryService::new(
                Default::default(),
            )),
            request_policy: Arc::new(AllowAllHostedRequestPolicy),
        },
    )
    .await
    .unwrap();

    let lease = pool.lease("tenant-idle").await.unwrap();
    assert!(
        !lease.server.runtime.allows_local_workspaces(),
        "the hosted profile must reach native workspace-tool enforcement"
    );
    drop(lease);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !pool.is_empty().await {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("gateway did not evict the idle tenant runtime");

    controller.stop().await.unwrap();
    let lease = pool.lease("tenant-after-shutdown").await.unwrap();
    drop(lease);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        pool.len().await,
        1,
        "the eviction loop must stop with the gateway"
    );
    pool.shutdown(std::time::Duration::ZERO).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_and_size_limits_fail_requests_deterministically() {
    let fixture = fixture(
        "limits",
        RateLimitConfig {
            burst: 3,
            per_second: 0.0001,
            max_request_bytes: 4096,
        },
        true,
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();

    let mut limited = false;
    for _ in 0..5 {
        let response = call(&mut socket, "hosted/whoami", serde_json::json!({})).await;
        if let Some(error) = response.error {
            assert!(error.message.contains("rate limit"), "{}", error.message);
            limited = true;
            break;
        }
    }
    assert!(limited, "burst of 3 must rate-limit the 4th request");

    // Oversized frames are rejected without parsing (the gateway answers
    // with a null-id error because it never reads the request id).
    let huge = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "big",
        "method": "hosted/whoami",
        "params": { "padding": "x".repeat(5000) },
    });
    socket
        .send(Message::Text(serde_json::to_string(&huge).unwrap().into()))
        .await
        .unwrap();
    let raw = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = raw else {
        panic!("expected text response");
    };
    assert!(text.contains("too large"), "{text}");

    fixture.controller.stop().await.unwrap();
}
