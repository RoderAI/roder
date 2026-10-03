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
