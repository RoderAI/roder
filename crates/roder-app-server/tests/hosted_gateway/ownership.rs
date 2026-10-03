use super::*;
use roder_core::RuntimeExecutionLease;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread")]
async fn gateway_closes_an_idle_socket_after_runtime_owner_revocation() {
    let authority = Arc::new(RuntimeExecutionLease::new(
        Instant::now() + Duration::from_secs(30),
    ));
    let factory_authority = authority.clone();
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("owner-loss"),
            ..Default::default()
        },
        Arc::new(move |_tenant, _dir| {
            let authority = factory_authority.clone();
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                Ok(Arc::new(AppServer::new(Arc::new(
                    Runtime::new(builder.build()?, RuntimeConfig::default())?
                        .with_execution_lease(authority),
                ))))
            })
        }),
    ));
    let fixture = fixture_with_pool(
        pool,
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let who = call(&mut socket, "hosted/whoami", serde_json::json!({})).await;
    assert!(who.error.is_none());
    authority.revoke();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(error)) => panic!("expected a clean owner-loss close: {error}"),
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .unwrap();
    // Reconnecting to this stale process must not restore the cached runtime.
    let mut retry = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), retry.next())
        .await
        .unwrap();
    assert!(matches!(result, Some(Ok(Message::Close(_))) | None));
    fixture.controller.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires isolated MySQL"]
async fn mysql_generation_loss_revokes_the_runtime_and_closes_its_socket() {
    use roder_api::extension::RoderExtension;
    use roder_app_server::hosted::HostedRuntimeLeaseBackend;
    use roder_ext_mysql_session::{
        MysqlSessionConfig, MysqlSessionExtension, MysqlSessionStore,
        ownership::{RuntimeOwnerClaim, RuntimeOwnerLease},
    };
    struct Backend {
        store: MysqlSessionStore,
        lease: RuntimeOwnerLease,
    }
    #[async_trait::async_trait]
    impl HostedRuntimeLeaseBackend for Backend {
        async fn release(&self) -> anyhow::Result<bool> { self.store.release_runtime_owner(&self.lease).await }
        async fn renew(&self, ttl: Duration) -> anyhow::Result<bool> {
            Ok(self
                .store
                .renew_runtime_owner(&self.lease, ttl)
                .await?
                .is_some())
        }
    }
    let url = std::env::var("RODER_MYSQL_TEST_URL").expect("isolated MySQL test URL required");
    let store = MysqlSessionStore::connect(
        &MysqlSessionConfig::new(url, format!("supervisor-tenant-a-{}", uuid::Uuid::new_v4()))
            .unwrap(),
    )
    .await
    .unwrap();
    let requested_at = Instant::now();
    let ttl = Duration::from_secs(3);
    let RuntimeOwnerClaim::Acquired(lease) = store
        .claim_runtime_owner(uuid::Uuid::new_v4(), "127.0.0.1:4500".parse().unwrap(), ttl)
        .await
        .unwrap()
    else {
        panic!("owner")
    };
    let authority = Arc::new(RuntimeExecutionLease::new(requested_at + ttl));
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(Arc::new(FakeInferenceEngine));
    MysqlSessionExtension::from_store(store.with_runtime_owner(&lease).unwrap())
        .install(&mut registry)
        .unwrap();
    let server = Arc::new(AppServer::new(Arc::new(
        Runtime::new(registry.build().unwrap(), RuntimeConfig::default())
            .unwrap()
            .with_execution_lease(authority.clone()),
    )));
    server
        .supervise_runtime_lease(
            authority.clone(),
            Arc::new(Backend {
                store: store.clone(),
                lease: lease.clone(),
            }),
            ttl,
        )
        .unwrap();
    let factory_server = server.clone();
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("mysql-owner"),
            ..Default::default()
        },
        Arc::new(move |_, _| {
            let server = factory_server.clone();
            Box::pin(async move { Ok(server) })
        }),
    ));
    let fixture = fixture_with_pool(
        pool,
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut socket, "hosted/whoami", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if store
                .runtime_owner()
                .await
                .unwrap()
                .unwrap()
                .expires_at_micros
                > lease.expires_at_micros
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    authority.require_live().unwrap();
    assert!(store.release_runtime_owner(&lease).await.unwrap());
    let RuntimeOwnerClaim::Acquired(replacement) = store
        .claim_runtime_owner(uuid::Uuid::new_v4(), "127.0.0.1:4501".parse().unwrap(), ttl)
        .await
        .unwrap()
    else {
        panic!("replacement")
    };
    assert!(replacement.generation > lease.generation);
    let closed = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await
        .unwrap();
    assert!(matches!(closed, Some(Ok(Message::Close(_))) | None));
    assert!(server.runtime.ensure_execution_authority().is_err());
    assert_eq!(
        store.runtime_owner().await.unwrap().unwrap().generation,
        replacement.generation
    );
    fixture.controller.stop().await.unwrap();
    assert!(store.release_runtime_owner(&replacement).await.unwrap());
}

#[tokio::test]
async fn reconnect_resolves_ownership_again_after_cached_runtime_revocation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let authorities = Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory_calls = calls.clone();
    let factory_authorities = authorities.clone();
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile { data_root: temp_dir("owner-recovery"), ..Default::default() },
        Arc::new(move |_, _| {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            let authority = Arc::new(RuntimeExecutionLease::new(
                std::time::Instant::now() + Duration::from_secs(30),
            ));
            factory_authorities.lock().unwrap().push(authority.clone());
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                let runtime = Runtime::new(builder.build()?, RuntimeConfig::default())?
                    .with_execution_lease(authority);
                Ok(Arc::new(AppServer::new(Arc::new(runtime))))
            })
        }),
    ));
    let fixture = fixture_with_pool(pool, RateLimitConfig::default(), Arc::new(AllowAllHostedRequestPolicy)).await;
    let mut old = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();
    assert!(call(&mut old, "hosted/whoami", serde_json::json!({})).await.error.is_none());
    authorities.lock().unwrap()[0].revoke();
    let mut replacement = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();
    assert!(call(&mut replacement, "hosted/whoami", serde_json::json!({})).await.error.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.pool.len().await, 1);
    assert!(authorities.lock().unwrap()[0].require_live().is_err());
    replacement.close(None).await.unwrap();
    fixture.controller.stop().await.unwrap();
}
