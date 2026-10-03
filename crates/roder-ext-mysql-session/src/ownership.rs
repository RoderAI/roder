//! Durable tenant-runtime ownership. These leases arbitrate owners; callers must
//! also fence runtime work and durable writes with the generation. A lease alone
//! cannot cancel an external action already in flight on a partitioned owner.

use crate::MysqlSessionStore;
use sqlx_core::row::Row;
use sqlx_mysql::{MySql, MySqlConnection};
use std::{net::SocketAddr, time::Duration};
use uuid::Uuid;

/// A database-issued ownership generation. Never persist credentials in this record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwnerLease {
    pub tenant_id: String,
    pub owner_id: Uuid,
    pub generation: u64,
    pub endpoint: SocketAddr,
    pub expires_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeOwnerClaim {
    Acquired(RuntimeOwnerLease),
    /// Includes repeat claims by the same process; claims do not renew implicitly.
    Occupied(RuntimeOwnerLease),
}

impl MysqlSessionStore {
    /// Derive a handle whose writes require this exact live generation.
    /// The database checks authority atomically on every mutation.
    pub fn with_runtime_owner(&self, lease: &RuntimeOwnerLease) -> anyhow::Result<Self> {
        self.validate_owner_lease(lease)?;
        let mut store = self.clone();
        store.owner = Some(lease.clone());
        Ok(store)
    }

    /// Claim a tenant only if no unexpired owner exists. The database clock owns
    /// expiry; generation rows are retained after release and never reset.
    pub async fn claim_runtime_owner(
        &self,
        owner_id: Uuid,
        endpoint: SocketAddr,
        ttl: Duration,
    ) -> anyhow::Result<RuntimeOwnerClaim> {
        let ttl = ttl_micros(ttl)?;
        let tenant = self.tenant_id().to_owned();
        validate_scope(&tenant)?;
        let pool = self.pool.clone();
        self.executor.run(async move {
            // Initialize outside the locking transaction to avoid upgrading the
            // shared duplicate-key lock of competing first-time claimers.
            sqlx_core::query::query::<MySql>(
                "INSERT IGNORE INTO roder_runtime_owners (tenant_id, generation, expires_at) VALUES (?, 0, 0)"
            ).bind(&tenant).execute(&pool).await?;
            let mut tx = pool.begin().await?;
            let row = sqlx_core::query::query::<MySql>(
                "SELECT owner_id, generation, endpoint, expires_at FROM roder_runtime_owners WHERE tenant_id = ? FOR UPDATE"
            ).bind(&tenant).fetch_one(&mut *tx).await?;
            // Read time after acquiring the row lock, not before a lock wait.
            let now = database_time(&mut tx).await?;
            let previous: u64 = row.try_get("generation")?;
            if row.try_get::<i64, _>("expires_at")? > now {
                let lease = decode(&tenant, &row)?;
                tx.commit().await?;
                return Ok(RuntimeOwnerClaim::Occupied(lease));
            }
            let generation = previous.checked_add(1).ok_or_else(|| anyhow::anyhow!("runtime owner generation exhausted"))?;
            let expires_at_micros = now.checked_add(ttl).ok_or_else(|| anyhow::anyhow!("runtime owner expiry overflow"))?;
            sqlx_core::query::query::<MySql>(
                "UPDATE roder_runtime_owners SET owner_id = ?, generation = ?, endpoint = ?, expires_at = ? WHERE tenant_id = ?"
            ).bind(owner_id.to_string()).bind(generation).bind(endpoint.to_string())
                .bind(expires_at_micros).bind(&tenant).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(RuntimeOwnerClaim::Acquired(RuntimeOwnerLease { tenant_id: tenant, owner_id, generation, endpoint, expires_at_micros }))
        }).await
    }

    /// Renew only a still-live, matching generation. Expired owners must claim a
    /// new generation, even when no competitor has acquired the tenant yet.
    pub async fn renew_runtime_owner(
        &self,
        lease: &RuntimeOwnerLease,
        ttl: Duration,
    ) -> anyhow::Result<Option<RuntimeOwnerLease>> {
        self.validate_owner_lease(lease)?;
        let ttl = ttl_micros(ttl)?;
        let pool = self.pool.clone();
        let lease = lease.clone();
        self.executor.run(async move {
            let mut tx = pool.begin().await?;
            let row = sqlx_core::query::query::<MySql>(
                "SELECT owner_id, generation, endpoint, expires_at FROM roder_runtime_owners WHERE tenant_id = ? FOR UPDATE"
            ).bind(&lease.tenant_id).fetch_optional(&mut *tx).await?;
            let now = database_time(&mut tx).await?;
            let Some(row) = row else { return Ok(None); };
            let current = decode(&lease.tenant_id, &row)?;
            if !same_owner(&current, &lease) || current.expires_at_micros <= now {
                return Ok(None);
            }
            let expires_at_micros = now.checked_add(ttl).ok_or_else(|| anyhow::anyhow!("runtime owner expiry overflow"))?.max(current.expires_at_micros);
            sqlx_core::query::query::<MySql>(
                "UPDATE roder_runtime_owners SET expires_at = ? WHERE tenant_id = ?"
            ).bind(expires_at_micros).bind(&lease.tenant_id).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(Some(RuntimeOwnerLease { expires_at_micros, ..current }))
        }).await
    }

    /// Release a live matching owner without erasing its fencing generation.
    /// The caller must quiesce work before releasing authority.
    pub async fn release_runtime_owner(&self, lease: &RuntimeOwnerLease) -> anyhow::Result<bool> {
        self.validate_owner_lease(lease)?;
        let pool = self.pool.clone();
        let lease = lease.clone();
        self.executor.run(async move {
            let result = sqlx_core::query::query::<MySql>(
                "UPDATE roder_runtime_owners SET expires_at = 0 WHERE tenant_id = ? AND owner_id = ? AND generation = ? AND endpoint = ? AND expires_at > TIMESTAMPDIFF(MICROSECOND, '1970-01-01', UTC_TIMESTAMP(6))"
            ).bind(&lease.tenant_id).bind(lease.owner_id.to_string()).bind(lease.generation)
                .bind(lease.endpoint.to_string()).execute(&pool).await?;
            Ok(result.rows_affected() == 1)
        }).await
    }

    /// Routing observation only: callers must not treat this snapshot as authority
    /// to execute. The owner can expire or be replaced immediately after reading.
    pub async fn runtime_owner(&self) -> anyhow::Result<Option<RuntimeOwnerLease>> {
        let tenant = self.tenant_id().to_owned();
        validate_scope(&tenant)?;
        let pool = self.pool.clone();
        self.executor.run(async move {
            let row = sqlx_core::query::query::<MySql>(
                "SELECT owner_id, generation, endpoint, expires_at FROM roder_runtime_owners WHERE tenant_id = ? AND expires_at > TIMESTAMPDIFF(MICROSECOND, '1970-01-01', UTC_TIMESTAMP(6))"
            ).bind(&tenant).fetch_optional(&pool).await?;
            row.map(|row| decode(&tenant, &row)).transpose()
        }).await
    }

    fn validate_owner_lease(&self, lease: &RuntimeOwnerLease) -> anyhow::Result<()> {
        anyhow::ensure!(
            lease.tenant_id == self.tenant_id(),
            "runtime owner lease belongs to another tenant"
        );
        validate_scope(self.tenant_id())
    }
}

fn same_owner(left: &RuntimeOwnerLease, right: &RuntimeOwnerLease) -> bool {
    left.owner_id == right.owner_id
        && left.generation == right.generation
        && left.endpoint == right.endpoint
}

fn validate_scope(tenant: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !tenant.is_empty() && tenant.len() <= 191,
        "runtime owner tenant id must be 1 to 191 bytes"
    );
    Ok(())
}

fn ttl_micros(ttl: Duration) -> anyhow::Result<i64> {
    anyhow::ensure!(
        ttl >= Duration::from_secs(1) && ttl <= Duration::from_secs(300),
        "runtime owner TTL must be between 1 and 300 seconds"
    );
    Ok(ttl.as_micros().try_into()?)
}

pub(crate) async fn database_time(connection: &mut MySqlConnection) -> anyhow::Result<i64> {
    Ok(sqlx_core::query_scalar::query_scalar::<MySql, i64>(
        "SELECT TIMESTAMPDIFF(MICROSECOND, '1970-01-01', UTC_TIMESTAMP(6))",
    )
    .fetch_one(connection)
    .await?)
}

fn decode(tenant: &str, row: &sqlx_mysql::MySqlRow) -> anyhow::Result<RuntimeOwnerLease> {
    Ok(RuntimeOwnerLease {
        tenant_id: tenant.to_owned(),
        owner_id: row.try_get::<String, _>("owner_id")?.parse()?,
        generation: row.try_get("generation")?,
        endpoint: row.try_get::<String, _>("endpoint")?.parse()?,
        expires_at_micros: row.try_get("expires_at")?,
    })
}
