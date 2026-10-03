//! Run with RODER_MYSQL_TEST_URL set to an isolated database and `--ignored`.
use roder_ext_mysql_session::ownership::{RuntimeOwnerClaim, RuntimeOwnerLease};
use roder_ext_mysql_session::{MysqlSessionConfig, MysqlSessionStore};
use std::{net::SocketAddr, time::Duration};
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(30);
fn endpoint(port: u16) -> SocketAddr {
    ([127, 0, 0, 1], port).into()
}
async fn store(tenant: &str) -> MysqlSessionStore {
    let url = std::env::var("RODER_MYSQL_TEST_URL").expect("isolated MySQL test URL required");
    MysqlSessionStore::connect(&MysqlSessionConfig::new(url, tenant).unwrap())
        .await
        .unwrap()
}
fn acquired(claim: RuntimeOwnerClaim) -> RuntimeOwnerLease {
    match claim {
        RuntimeOwnerClaim::Acquired(lease) => lease,
        RuntimeOwnerClaim::Occupied(_) => panic!("expected new owner"),
    }
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn concurrent_replicas_have_exactly_one_owner() {
    let tenant = format!("owner-race-{}", Uuid::new_v4());
    let first = store(&tenant).await;
    let second = store(&tenant).await;
    let (left, right) = tokio::join!(
        first.claim_runtime_owner(Uuid::new_v4(), endpoint(4500), TTL),
        second.claim_runtime_owner(Uuid::new_v4(), endpoint(4501), TTL),
    );
    let (winner, observed) = match (left.unwrap(), right.unwrap()) {
        (RuntimeOwnerClaim::Acquired(a), RuntimeOwnerClaim::Occupied(b))
        | (RuntimeOwnerClaim::Occupied(b), RuntimeOwnerClaim::Acquired(a)) => (a, b),
        other => panic!("one winner required: {other:?}"),
    };
    assert_eq!(winner, observed);
    assert_eq!(winner.generation, 1);
    assert_eq!(first.runtime_owner().await.unwrap(), Some(winner.clone()));
    assert_eq!(
        second
            .claim_runtime_owner(winner.owner_id, winner.endpoint, TTL)
            .await
            .unwrap(),
        RuntimeOwnerClaim::Occupied(winner.clone())
    );
    assert!(first.release_runtime_owner(&winner).await.unwrap());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn expired_owner_cannot_renew_or_release_its_replacement() {
    let tenant = format!("owner-expiry-{}", Uuid::new_v4());
    let first = store(&tenant).await;
    let old = acquired(
        first
            .claim_runtime_owner(Uuid::new_v4(), endpoint(4500), Duration::from_secs(1))
            .await
            .unwrap(),
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(first.runtime_owner().await.unwrap().is_none());
    assert!(
        first
            .renew_runtime_owner(&old, TTL)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!first.release_runtime_owner(&old).await.unwrap());
    let second = store(&tenant).await;
    let replacement = acquired(
        second
            .claim_runtime_owner(Uuid::new_v4(), endpoint(4501), TTL)
            .await
            .unwrap(),
    );
    assert_eq!(replacement.generation, old.generation + 1);
    assert!(
        first
            .renew_runtime_owner(&old, TTL)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!first.release_runtime_owner(&old).await.unwrap());
    assert_eq!(
        second.runtime_owner().await.unwrap(),
        Some(replacement.clone())
    );
    assert!(second.release_runtime_owner(&replacement).await.unwrap());
    assert!(second.runtime_owner().await.unwrap().is_none());
    let reclaimed = acquired(
        first
            .claim_runtime_owner(old.owner_id, old.endpoint, TTL)
            .await
            .unwrap(),
    );
    assert_eq!(reclaimed.generation, replacement.generation + 1);
    assert!(!first.release_runtime_owner(&old).await.unwrap());
    assert!(first.release_runtime_owner(&reclaimed).await.unwrap());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn renewal_preserves_generation_and_tenant_scope_is_case_sensitive() {
    let tenant = format!("owner-scope-{}", Uuid::new_v4());
    let first = store(&tenant).await;
    let other = first.for_tenant(&tenant.to_uppercase()).unwrap();
    let lease = acquired(
        first
            .claim_runtime_owner(Uuid::new_v4(), endpoint(4500), TTL)
            .await
            .unwrap(),
    );
    let other_lease = acquired(
        other
            .claim_runtime_owner(Uuid::new_v4(), endpoint(4501), TTL)
            .await
            .unwrap(),
    );
    assert_eq!(other_lease.generation, 1);
    assert!(other.renew_runtime_owner(&lease, TTL).await.is_err());
    assert!(other.release_runtime_owner(&lease).await.is_err());
    let renewed = first
        .renew_runtime_owner(&lease, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renewed.generation, lease.generation);
    assert!(renewed.expires_at_micros > lease.expires_at_micros);
    let shorter = first
        .renew_runtime_owner(&renewed, Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(shorter.expires_at_micros, renewed.expires_at_micros);
    assert!(first.release_runtime_owner(&renewed).await.unwrap());
    assert!(other.release_runtime_owner(&other_lease).await.unwrap());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn invalid_budgets_and_wrong_process_identity_do_not_change_authority() {
    let first = store(&format!("owner-guard-{}", Uuid::new_v4())).await;
    let id = Uuid::new_v4();
    assert!(
        first
            .claim_runtime_owner(id, endpoint(4500), Duration::ZERO)
            .await
            .is_err()
    );
    assert!(
        first
            .claim_runtime_owner(id, endpoint(4500), Duration::from_secs(301))
            .await
            .is_err()
    );
    assert!(first.runtime_owner().await.unwrap().is_none());
    let lease = acquired(
        first
            .claim_runtime_owner(id, endpoint(4500), TTL)
            .await
            .unwrap(),
    );
    let forged = RuntimeOwnerLease {
        owner_id: Uuid::new_v4(),
        ..lease.clone()
    };
    assert!(
        first
            .renew_runtime_owner(&forged, TTL)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!first.release_runtime_owner(&forged).await.unwrap());
    assert_eq!(first.runtime_owner().await.unwrap(), Some(lease.clone()));
    assert!(first.release_runtime_owner(&lease).await.unwrap());
}
