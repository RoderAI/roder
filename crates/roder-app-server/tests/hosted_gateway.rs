//! Hosted gateway e2e over loopback WebSocket (phase 72, Task 2): auth
//! before dispatch, query-token rejection, hosted/whoami + service-account
//! lifecycle, tenant thread isolation, notification filtering, and rate
//! limits. Fully offline with the fake inference engine.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::identity::{HostedRole, HostedScope, PrincipalContext, TenantContext};
use roder_app_server::AppServer;
use roder_app_server::hosted::auth::PrincipalSeed;
use roder_app_server::hosted::{
    AuditLog, HostedAuthenticator, HostedGatewayOptions, RateLimitConfig, TenantRegistry,
    serve_hosted_gateway,
};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_protocol::{JsonRpcRequest, JsonRpcResponse};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-hosted-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_server(label: &str) -> Arc<AppServer> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: temp_dir(label),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    Arc::new(AppServer::new(runtime))
}

struct Fixture {
    controller: roder_app_server::hosted::HostedGatewayController,
    authenticator: Arc<HostedAuthenticator>,
    url: String,
}

async fn fixture(label: &str, limits: RateLimitConfig) -> Fixture {
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
        app_server(label),
        HostedGatewayOptions {
            listen: "127.0.0.1:0".to_string(),
            authenticator: authenticator.clone(),
            tenants,
            audit: Arc::new(AuditLog::default()),
            limits,
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
        .send(Message::Text(serde_json::to_string(&request).unwrap().into()))
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
    let fixture = fixture("whoami", RateLimitConfig::default()).await;

    // Bad token: handshake fails before any dispatch.
    assert!(connect(&fixture.url, "rk_test_wrong").await.is_err());
    // Query-string credentials are rejected outright.
    let query_url = format!("{}/?token=rk_test_tenant_a_writer", fixture.url);
    let request = query_url.into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());

    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();
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
    let token = minted.result.as_ref().unwrap()["token"].as_str().unwrap().to_string();
    let key_id = minted.result.as_ref().unwrap()["keyId"].as_str().unwrap().to_string();

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
    assert!(!records.contains(&token), "audit must never contain raw tokens");

    let _ = fixture.authenticator;
    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tenants_cannot_touch_or_observe_each_others_threads() {
    let fixture = fixture("isolation", RateLimitConfig::default()).await;
    let mut tenant_a = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();
    let mut tenant_b = connect(&fixture.url, "rk_test_tenant_b_writer").await.unwrap();
    for socket in [&mut tenant_a, &mut tenant_b] {
        assert!(call(socket, "initialize", serde_json::json!({})).await.error.is_none());
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
    let thread_id = started.result.unwrap()["thread"]["id"].as_str().unwrap().to_string();

    // Tenant B cannot read, steer, or archive tenant A's thread.
    for method in ["thread/read", "thread/archive"] {
        let response = call(
            &mut tenant_b,
            method,
            serde_json::json!({ "threadId": thread_id }),
        )
        .await;
        let error = response.error.expect(method);
        assert!(error.message.contains("wrong_tenant"), "{method}: {}", error.message);
    }

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
    assert!(a_saw_notification, "owner tenant must receive thread notifications");

    // Tenant B's socket stays silent (no notifications for A's thread).
    let quiet =
        tokio::time::timeout(std::time::Duration::from_millis(500), tenant_b.next()).await;
    assert!(quiet.is_err(), "tenant B must not receive tenant A notifications");

    fixture.controller.stop().await.unwrap();
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
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();

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
