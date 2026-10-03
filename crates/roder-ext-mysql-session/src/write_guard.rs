//! Serializes owner changes with durable mutations, including legacy/unfenced handles.
use crate::ownership::RuntimeOwnerLease;
use sqlx_core::{pool::Pool, row::Row, transaction::Transaction};
use sqlx_mysql::MySql;

pub(crate) async fn begin(
    pool: &Pool<MySql>,
    tenant: &str,
    owner: Option<&RuntimeOwnerLease>,
) -> anyhow::Result<Transaction<'static, MySql>> {
    anyhow::ensure!(
        !tenant.is_empty() && tenant.len() <= 191,
        "runtime owner tenant id must be 1 to 191 bytes"
    );
    let mut tx = pool.begin().await?;
    // Establish an exclusive row lock even for a previously unowned tenant.
    // This also closes the first-claim race under READ COMMITTED isolation.
    sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_runtime_owners (tenant_id, generation, expires_at) VALUES (?, 0, 0) ON DUPLICATE KEY UPDATE tenant_id = VALUES(tenant_id)"
    ).bind(tenant).execute(&mut *tx).await?;
    let row = sqlx_core::query::query::<MySql>(
        "SELECT owner_id, generation, endpoint, expires_at FROM roder_runtime_owners WHERE tenant_id = ?"
    ).bind(tenant).fetch_one(&mut *tx).await?;
    let generation: u64 = row.try_get("generation")?;
    if generation == 0 && owner.is_none() {
        return Ok(tx);
    }
    let owner =
        owner.ok_or_else(|| anyhow::anyhow!("runtime ownership required for session write"))?;
    let now = crate::ownership::database_time(&mut tx).await?;
    anyhow::ensure!(
        owner.tenant_id == tenant
            && owner.generation == generation
            && row.try_get::<Option<String>, _>("owner_id")?.as_deref()
                == Some(owner.owner_id.to_string().as_str())
            && row.try_get::<Option<String>, _>("endpoint")?.as_deref()
                == Some(owner.endpoint.to_string().as_str())
            && row.try_get::<i64, _>("expires_at")? > now,
        "runtime ownership lost before session write"
    );
    // Hold the owner lock through the mutation and commit. A new generation
    // cannot be granted between this check and the durable write.
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MysqlSessionConfig, MysqlSessionStore, ownership::RuntimeOwnerClaim};
    use std::time::Duration;
    use uuid::Uuid;

    #[tokio::test]
    #[ignore = "requires isolated MySQL"]
    async fn a_new_generation_waits_for_an_already_authorized_write_to_commit() {
        let url = std::env::var("RODER_MYSQL_TEST_URL").expect("isolated MySQL test URL required");
        let store = MysqlSessionStore::connect(
            &MysqlSessionConfig::new(url, format!("write-lock-{}", Uuid::new_v4())).unwrap(),
        )
        .await
        .unwrap();
        let RuntimeOwnerClaim::Acquired(lease) = store
            .claim_runtime_owner(
                Uuid::new_v4(),
                "127.0.0.1:4500".parse().unwrap(),
                Duration::from_secs(1),
            )
            .await
            .unwrap()
        else {
            panic!("owner")
        };
        let pool = store.pool.clone();
        let contender = store.clone();
        store.executor.run(async move {
            let mut tx = begin(&pool, &lease.tenant_id, Some(&lease)).await?;
            tokio::time::sleep(Duration::from_millis(1100)).await;
            let mut claim = tokio::spawn(async move {
                contender.claim_runtime_owner(Uuid::new_v4(), "127.0.0.1:4501".parse().unwrap(), Duration::from_secs(30)).await
            });
            // The old lease is expired, but its already-authorized transaction
            // still serializes the next generation behind its final write.
            assert!(tokio::time::timeout(Duration::from_millis(100), &mut claim).await.is_err());
            sqlx_core::query::query::<MySql>(
                "INSERT INTO roder_session_extension_state (tenant_id, thread_id, record, created_at) VALUES (?, 'thread', '{}', 0)"
            ).bind(&lease.tenant_id).execute(&mut *tx).await?;
            tx.commit().await?;
            let RuntimeOwnerClaim::Acquired(next) = tokio::time::timeout(Duration::from_secs(3), claim).await??? else { panic!("next owner") };
            assert_eq!(next.generation, lease.generation + 1);
            assert!(begin(&pool, &lease.tenant_id, Some(&lease)).await.is_err());
            Ok(())
        }).await.unwrap();
    }
}
