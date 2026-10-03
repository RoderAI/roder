use super::*;
use roder_app_server::hosted::HostedRuntimeRedirect;
use std::{net::SocketAddr, sync::Mutex, time::Duration};

fn redirect_pool(
    endpoint: Arc<Mutex<SocketAddr>>,
    calls: Arc<AtomicUsize>,
) -> Arc<HostedRuntimePool> {
    Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("redirect"),
            ..Default::default()
        },
        Arc::new(move |tenant_id, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            let endpoint = *endpoint.lock().unwrap();
            Box::pin(async move {
                Err(HostedRuntimeRedirect {
                    tenant_id,
                    endpoint,
                }
                .into())
            })
        }),
    ))
}

async fn expect_closed(socket: &mut Socket) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("owner relay must close");
}

fn register_rotated_key(auth: &HostedAuthenticator, tenant: &str) {
    auth.register_static_key(
        "rk_test_rotated_owner",
        PrincipalSeed {
            tenant_id: tenant.into(),
            principal: PrincipalContext::User {
                user_id: "user-tenant-a".into(),
                display_name: None,
            },
            role: HostedRole::TenantAdmin,
            scopes: vec![HostedScope::Read, HostedScope::Write, HostedScope::Admin],
        },
    )
    .unwrap();
}

#[tokio::test]
async fn reconnect_and_rotated_credentials_reach_the_same_owner_without_local_runtime() {
    let owner = fixture("owner", RateLimitConfig::default(), false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let edge = fixture_with_pool(
        redirect_pool(
            Arc::new(Mutex::new(owner.controller.listen_addr)),
            calls.clone(),
        ),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    register_rotated_key(&owner.authenticator, "tenant-a");
    register_rotated_key(&edge.authenticator, "tenant-a");
    for token in ["rk_test_tenant_a_writer", "rk_test_rotated_owner"] {
        let mut socket = connect(&edge.url, token).await.unwrap();
        let result = call(&mut socket, "hosted/whoami", serde_json::json!({})).await;
        assert!(result.error.is_none(), "{:?}", result.error);
        socket.close(None).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(owner.pool.len().await, 1);
    assert_eq!(edge.pool.len().await, 0);
    // Closing the owner tears down the relay rather than silently reconnecting.
    let mut socket = connect(&edge.url, "rk_test_rotated_owner").await.unwrap();
    assert!(
        call(&mut socket, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    owner.controller.stop().await.unwrap();
    expect_closed(&mut socket).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    edge.controller.stop().await.unwrap();
}

#[tokio::test]
async fn owner_routing_stops_after_one_hop() {
    let endpoint = Arc::new(Mutex::new("127.0.0.1:1".parse().unwrap()));
    let calls = Arc::new(AtomicUsize::new(0));
    let edge = fixture_with_pool(
        redirect_pool(endpoint.clone(), calls.clone()),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    *endpoint.lock().unwrap() = edge.controller.listen_addr;
    let mut socket = connect(&edge.url, "rk_test_tenant_a_writer").await.unwrap();
    expect_closed(&mut socket).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one relay, no redirect loop"
    );
    edge.controller.stop().await.unwrap();
}

#[tokio::test]
async fn replicas_cannot_disagree_about_the_forwarded_credentials_tenant() {
    let owner = fixture("owner-mismatch", RateLimitConfig::default(), false).await;
    let edge = fixture_with_pool(
        redirect_pool(
            Arc::new(Mutex::new(owner.controller.listen_addr)),
            Arc::new(AtomicUsize::new(0)),
        ),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    register_rotated_key(&edge.authenticator, "tenant-a");
    register_rotated_key(&owner.authenticator, "tenant-b");
    let mut socket = connect(&edge.url, "rk_test_rotated_owner").await.unwrap();
    expect_closed(&mut socket).await;
    assert_eq!(
        owner.pool.len().await,
        0,
        "reject before constructing the wrong tenant runtime"
    );
    edge.controller.stop().await.unwrap();
    owner.controller.stop().await.unwrap();
}

#[tokio::test]
async fn retiring_relay_preserves_browser_execution_then_reconnects_through_another_replica() {
    use roder_app_server::hosted::HostedRuntimeLeaseBackend;
    use roder_core::RuntimeExecutionLease;
    use std::time::Instant;
    struct Backend(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl HostedRuntimeLeaseBackend for Backend {
        async fn renew(&self, _: Duration) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn release(&self) -> anyhow::Result<bool> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }
    }
    let releases = Arc::new(AtomicUsize::new(0));
    let factory_releases = releases.clone();
    let owner_pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("relay-owner"),
            allow_local_workspaces: true,
            ..Default::default()
        },
        Arc::new(move |_, directory| {
            let releases = factory_releases.clone();
            Box::pin(async move {
                let authority = Arc::new(RuntimeExecutionLease::new(
                    Instant::now() + Duration::from_secs(30),
                ));
                let mut registry = ExtensionRegistryBuilder::new();
                registry.inference_engine(Arc::new(FakeInferenceEngine));
                registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
                    base_path: directory.join("threads"),
                }));
                let runtime = Runtime::new(registry.build()?, RuntimeConfig::default())?
                    .with_execution_lease(authority.clone());
                let server = Arc::new(AppServer::new(Arc::new(runtime)));
                server.supervise_runtime_lease(
                    authority,
                    Arc::new(Backend(releases)),
                    Duration::from_secs(30),
                )?;
                Ok(server)
            })
        }),
    ));
    let owner = fixture_with_pool(
        owner_pool,
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let endpoint = Arc::new(Mutex::new(owner.controller.listen_addr));
    let retiring = fixture_with_pool(
        redirect_pool(endpoint.clone(), Arc::new(AtomicUsize::new(0))),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let replacement = fixture_with_pool(
        redirect_pool(endpoint, Arc::new(AtomicUsize::new(0))),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let mut socket = connect(&retiring.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let directory = temp_dir("relay-workspace");
    let workspace = call(
        &mut socket,
        "workspace/create",
        serde_json::json!({
            "roots":[{"path":directory}],"defaultRootPath":directory
        }),
    )
    .await
    .result
    .unwrap()["workspace"]
        .clone();
    let thread = call(&mut socket, "thread/start", serde_json::json!({
        "workspaceId":workspace["id"],"rootId":workspace["defaultRootId"],"model":"mock",
        "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
    })).await.result.unwrap()["thread"]["id"].clone();
    let executor = call(
        &mut socket,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    let old_owner = owner.pool.lease("tenant-a").await.unwrap();
    let mut events = old_owner.server.runtime.subscribe_events();
    assert!(
        call(
            &mut socket,
            "turn/start",
            serde_json::json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"})
        )
        .await
        .error
        .is_none()
    );
    let execution = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                panic!("socket closed before tool request")
            };
            let event: serde_json::Value = serde_json::from_str(&text).unwrap();
            if event["method"] == "thread/toolExecutionRequested" {
                break event["params"].clone();
            }
        }
    })
    .await
    .unwrap();
    retiring.pool.begin_owner_drain().await;
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert_eq!(
        releases.load(Ordering::SeqCst),
        0,
        "pending browser action must retain ownership"
    );
    assert_eq!(old_owner.server.runtime.active_turn_count().await, 1);
    assert_eq!(
        retiring
            .pool
            .poll_owner_drain()
            .await
            .unwrap()
            .remaining_relays,
        1
    );
    retiring.pool.resume_owner_admission().await;
    tokio::time::sleep(Duration::from_millis(750)).await;
    old_owner
        .server
        .runtime
        .ensure_execution_authority()
        .unwrap();
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    retiring.pool.begin_owner_drain().await;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let rejected = call(
        &mut socket,
        "turn/start",
        serde_json::json!({"threadId":thread,"prompt":"new work"}),
    )
    .await;
    assert_eq!(rejected.error.unwrap().code, -32015);
    assert!(call(&mut socket, "tools/resolve", serde_json::json!({
        "executor":executor,"turnId":execution["turnId"],"requestId":execution["requestId"],"output":"ok","isError":false
    })).await.error.is_none());
    tokio::time::timeout(Duration::from_secs(5), async {
        while events.recv().await.unwrap().kind != "turn.completed" {}
    })
    .await
    .unwrap();
    expect_closed(&mut socket).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while retiring
            .pool
            .poll_owner_drain()
            .await
            .unwrap()
            .remaining_relays
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(
        old_owner
            .server
            .runtime
            .ensure_execution_authority()
            .is_err()
    );
    let mut reconnected = connect(&replacement.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let history = call(
        &mut reconnected,
        "thread/read",
        serde_json::json!({"threadId":thread,"includeTurns":true}),
    )
    .await;
    assert!(history.error.is_none(), "{:?}", history.error);
    assert_eq!(history.result.unwrap()["thread"]["id"], thread);
    reconnected.close(None).await.unwrap();
    retiring.controller.stop().await.unwrap();
    replacement.controller.stop().await.unwrap();
    owner.controller.stop().await.unwrap();
}

#[tokio::test]
async fn owner_drain_requires_a_forwarded_connection_and_write_scope() {
    let owner = fixture("owner-drain-auth", RateLimitConfig::default(), false).await;
    let edge = fixture_with_pool(
        redirect_pool(
            Arc::new(Mutex::new(owner.controller.listen_addr)),
            Arc::new(AtomicUsize::new(0)),
        ),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let mut direct = connect(&owner.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let denied = call(
        &mut direct,
        "hosted/owner/drain",
        serde_json::json!({"draining":true}),
    )
    .await;
    assert_eq!(denied.error.unwrap().code, -32012);
    for auth in [&owner.authenticator, &edge.authenticator] {
        auth.register_static_key(
            "rk_test_relay_reader",
            PrincipalSeed {
                tenant_id: "tenant-a".into(),
                principal: PrincipalContext::User {
                    user_id: "reader".into(),
                    display_name: None,
                },
                role: HostedRole::Member,
                scopes: vec![HostedScope::Read],
            },
        )
        .unwrap();
    }
    let mut reader = connect(&edge.url, "rk_test_relay_reader").await.unwrap();
    let denied = call(
        &mut reader,
        "hosted/owner/drain",
        serde_json::json!({"draining":true}),
    )
    .await;
    assert_eq!(denied.error.unwrap().code, -32012);
    edge.pool.begin_owner_drain().await;
    expect_closed(&mut reader).await;
    assert!(call(&mut direct, "hosted/whoami", serde_json::json!({})).await.error.is_none());
    direct.close(None).await.unwrap();
    edge.controller.stop().await.unwrap();
    owner.controller.stop().await.unwrap();
}
