//! Hosted gateway e2e over loopback WebSocket (phase 72, Tasks 2–3): auth
//! before dispatch, query-token rejection, hosted/whoami + service-account
//! lifecycle, per-tenant runtime isolation (stores and notifications),
//! hosted workspace policy, idle eviction, and rate limits. Fully offline
//! with the fake inference engine.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::identity::{HostedRole, HostedScope, PrincipalContext, TenantContext};
use roder_app_server::AppServer;
use roder_app_server_hosted::auth::PrincipalSeed;
use roder_app_server_hosted::{
    AuditLog, HostedAuthenticator, HostedGatewayOptions, HostedRuntimePool, HostedRuntimeProfile,
    RateLimitConfig, TenantRegistry, serve_hosted_gateway,
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
    Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir(label),
            allow_local_workspaces,
            idle_ttl: std::time::Duration::from_secs(3600),
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
    controller: roder_app_server_hosted::HostedGatewayController,
    authenticator: Arc<HostedAuthenticator>,
    url: String,
}

async fn fixture(label: &str, limits: RateLimitConfig, allow_local_workspaces: bool) -> Fixture {
    let authenticator = Arc::new(HostedAuthenticator::default());
    let tenants = Arc::new(TenantRegistry::default());
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
            audit: Arc::new(AuditLog::default()),
            limits,
            hooks: Arc::new(roder_app_server_hosted::HookStore::default()),
            hook_delivery: Arc::new(roder_app_server_hosted::HookDeliveryService::new(
                Default::default(),
            )),
        },
    )
    .await
    .unwrap();
    let url = format!("ws://{}", controller.listen_addr);
    Fixture {
        controller,
        authenticator,
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
        serde_json::json!({ "threadId": thread_id, "prompt": "hello" }),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);

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
