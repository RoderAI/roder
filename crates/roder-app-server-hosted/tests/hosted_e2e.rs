//! Consolidated hosted multi-tenant e2e (phase 72, Task 7): one fake-auth
//! hosted gateway, two tenants, two users, a service account, fake runner
//! execution (mock engine), and fake hook delivery — exercised in a single
//! flow over loopback WebSocket. Granular per-surface proofs live in
//! `hosted_auth.rs`, `hosted_gateway.rs`, and `hosted_hooks.rs`; this test
//! proves the pieces compose. Fully offline.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::{SinkExt, StreamExt};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::identity::{HostedRole, HostedScope, PrincipalContext, TenantContext};
use roder_app_server::AppServer;
use roder_app_server_hosted::auth::PrincipalSeed;
use roder_app_server_hosted::{
    AuditLog, HookDeliveryService, HookStore, HostedAuthenticator, HostedGatewayOptions,
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
    let dir = std::env::temp_dir().join(format!("roder-he2e-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
    socket
        .send(Message::Text(
            serde_json::to_string(&JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(method)),
                method: method.to_string(),
                params: Some(params),
            })
            .unwrap()
            .into(),
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
            if value.get("id").is_some_and(|id| !id.is_null()) {
                return serde_json::from_value(value).unwrap();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hosted_e2e_two_tenants_service_account_hooks_and_isolation() {
    // Fake hook target counting hits.
    let hook_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_url = format!("http://{}/hook", hook_listener.local_addr().unwrap());
    let hook_hits = Arc::new(AtomicUsize::new(0));
    let task_hits = hook_hits.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((mut stream, _)) = hook_listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer).await;
            task_hits.fetch_add(1, Ordering::SeqCst);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await;
        }
    });

    // Gateway: two tenants, two users (tenant-admin each), fake runtimes.
    let authenticator = Arc::new(HostedAuthenticator::default());
    let tenants = Arc::new(TenantRegistry::default());
    for tenant in ["tenant-a", "tenant-b"] {
        tenants.insert(TenantContext {
            tenant_id: tenant.to_string(),
            display_name: None,
        });
        authenticator
            .register_static_key(
                &format!("rk_test_{}_admin", tenant.replace('-', "_")),
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
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("pool"),
            allow_local_workspaces: true, // test profile opt-in (fake local runner)
            idle_ttl: std::time::Duration::from_secs(3600),
        },
        Arc::new(|_tenant, data_dir| {
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
    let controller = serve_hosted_gateway(
        pool,
        HostedGatewayOptions {
            listen: "127.0.0.1:0".to_string(),
            authenticator: authenticator.clone(),
            tenants,
            audit: Arc::new(AuditLog::default()),
            limits: RateLimitConfig::default(),
            hooks: Arc::new(HookStore::default()),
            hook_delivery: Arc::new(HookDeliveryService::new(Default::default())),
        },
    )
    .await
    .unwrap();
    let url = format!("ws://{}", controller.listen_addr);

    let mut admin_a = connect(&url, "rk_test_tenant_a_admin").await.unwrap();
    let mut admin_b = connect(&url, "rk_test_tenant_b_admin").await.unwrap();
    for socket in [&mut admin_a, &mut admin_b] {
        assert!(
            call(socket, "initialize", serde_json::json!({}))
                .await
                .error
                .is_none()
        );
    }

    // Tenant A registers a hook for thread lifecycle events.
    let created = call(
        &mut admin_a,
        "hosted/hooks/create",
        serde_json::json!({ "hook": {
            "id": "e2e-hook",
            "scope": "tenant",
            "eventKinds": ["thread.", "turn."],
            "url": hook_url,
            "enabled": true,
            "createdAt": "1970-01-01T00:00:00Z",
            "updatedAt": "1970-01-01T00:00:00Z",
        }}),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    // Tenant B sees no hooks (tenant-scoped store).
    let b_hooks = call(&mut admin_b, "hosted/hooks/list", serde_json::json!({})).await;
    assert_eq!(
        b_hooks.result.unwrap()["hooks"].as_array().map(Vec::len),
        Some(0)
    );

    // Tenant A mints a service account; it authenticates into tenant A.
    let minted = call(
        &mut admin_a,
        "hosted/service_accounts/create",
        serde_json::json!({ "displayName": "e2e-sa" }),
    )
    .await;
    let sa_token = minted.result.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let mut service_account = connect(&url, &sa_token).await.unwrap();
    let sa_whoami = call(&mut service_account, "hosted/whoami", serde_json::json!({})).await;
    assert_eq!(sa_whoami.result.unwrap()["tenant"]["tenantId"], "tenant-a");
    // The member service account cannot administer hooks.
    let denied = call(
        &mut service_account,
        "hosted/hooks/list",
        serde_json::json!({}),
    )
    .await;
    assert!(
        denied
            .error
            .unwrap()
            .message
            .contains("tenant_admin_required")
    );

    // Full thread + turn lifecycle on tenant A's runtime (fake engine =
    // the fake/local runner execution profile).
    let workspace = temp_dir("ws-a");
    let created = call(
        &mut admin_a,
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
        &mut admin_a,
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
    // The service account (same tenant) can see the thread.
    let sa_list = call(&mut service_account, "thread/list", serde_json::json!({})).await;
    assert_eq!(
        sa_list.result.unwrap()["data"].as_array().map(Vec::len),
        Some(1)
    );
    // Tenant B cannot.
    let b_list = call(&mut admin_b, "thread/list", serde_json::json!({})).await;
    assert_eq!(
        b_list.result.unwrap()["data"].as_array().map(Vec::len),
        Some(0)
    );

    let turn = call(
        &mut admin_a,
        "turn/start",
        serde_json::json!({ "threadId": thread_id, "prompt": "hello" }),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);

    // Tenant A observes its turn notifications; tenant B stays silent.
    let mut a_notified = false;
    for _ in 0..50 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), admin_a.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value.get("method").is_some()
                    && value["params"]["threadId"].as_str() == Some(thread_id.as_str())
                {
                    a_notified = true;
                    break;
                }
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(a_notified);
    let b_quiet = tokio::time::timeout(std::time::Duration::from_millis(400), admin_b.next()).await;
    assert!(b_quiet.is_err(), "tenant B must observe nothing");

    // Hook deliveries fired for tenant A's thread/turn lifecycle.
    let mut delivered = false;
    for _ in 0..40 {
        if hook_hits.load(Ordering::SeqCst) >= 1 {
            delivered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(delivered, "tenant hook must receive lifecycle deliveries");

    // Revoking the service account cuts off new connections.
    let key_id = sa_token
        .strip_prefix("rk_sa_")
        .and_then(|rest| rest.split_once('.'))
        .map(|(key_id, _)| key_id.to_string())
        .unwrap();
    let revoked = call(
        &mut admin_a,
        "hosted/service_accounts/revoke",
        serde_json::json!({ "keyId": key_id }),
    )
    .await;
    assert_eq!(revoked.result.unwrap()["revoked"], true);
    assert!(connect(&url, &sa_token).await.is_err());

    controller.stop().await.unwrap();
}
