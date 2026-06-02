use anyhow::Result;
use roder_api::automations::{
    AutomationLeaseRecord, AutomationOccurrenceKey, AutomationRunId, AutomationServerId,
    AutomationServerRole,
};
use time::OffsetDateTime;

use crate::store::{AutomationStore, delete_expired_lease, delete_lease, insert_lease};

impl AutomationStore {
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_lease(
        &self,
        run_id: AutomationRunId,
        automation_id: String,
        occurrence_key: AutomationOccurrenceKey,
        server_id: AutomationServerId,
        server_role: AutomationServerRole,
        now: OffsetDateTime,
        lease_seconds: u64,
    ) -> Result<Option<AutomationLeaseRecord>> {
        delete_expired_lease(self.connection(), &occurrence_key, now)?;
        let lease = AutomationLeaseRecord {
            run_id,
            automation_id,
            occurrence_key,
            server_id,
            server_role,
            leased_at: now,
            expires_at: now + time::Duration::seconds(lease_seconds as i64),
        };
        match insert_lease(self.connection(), &lease) {
            Ok(_) => Ok(Some(lease)),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn renew_lease(
        &self,
        run_id: &AutomationRunId,
        server_id: &AutomationServerId,
        now: OffsetDateTime,
        lease_seconds: u64,
    ) -> Result<bool> {
        let updated = self.connection().execute(
            r#"
            UPDATE automation_leases
            SET leased_at = ?3, expires_at = ?4
            WHERE run_id = ?1 AND server_id = ?2 AND expires_at > ?3
            "#,
            rusqlite::params![
                run_id,
                server_id,
                now.unix_timestamp(),
                (now + time::Duration::seconds(lease_seconds as i64)).unix_timestamp(),
            ],
        )?;
        Ok(updated == 1)
    }

    pub fn release_lease(&self, run_id: &AutomationRunId) -> Result<bool> {
        Ok(delete_lease(self.connection(), run_id)? == 1)
    }
}
