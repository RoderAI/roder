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
