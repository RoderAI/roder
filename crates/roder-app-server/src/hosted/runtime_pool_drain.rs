//! Admission and confirmed release for a planned replica drain.
use super::*;
use roder_protocol::methods::{AppServerSideEffect, app_server_method_specs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedOwnerDrainStatus {
    /// Runtimes whose durable ownership has not yet been released.
    pub remaining_tenants: usize,
    /// Forwarded sockets still using this replica. They must also drain.
    pub remaining_relays: usize,
    /// Confirmed releases during this poll.
    pub released_tenants: usize,
}

pub(crate) struct RequestAdmission(Arc<AtomicUsize>);
impl Drop for RequestAdmission {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl HostedRuntimePool {
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Stops constructing new tenant runtimes and admitting new client work.
    /// Reconnects to resident owners and completion/recovery messages remain
    /// available so active work can finish. Internal child turns are unaffected.
    pub async fn begin_owner_drain(&self) {
        let _operation = self.drain_operation.lock().await;
        let _tenants = self.tenants.lock().await;
        self.draining.store(true, Ordering::Release);
    }

    /// Rollback restores client admission. Released owners are reconstructed
    /// through the factory; revoked guards are never resurrected.
    pub async fn resume_owner_admission(&self) {
        let _operation = self.drain_operation.lock().await;
        let _tenants = self.tenants.lock().await;
        self.draining.store(false, Ordering::Release);
    }

    pub(crate) async fn admit_owner_relay(&self) -> anyhow::Result<RequestAdmission> {
        let _tenants = self.tenants.lock().await;
        anyhow::ensure!(!self.is_draining(), "replica is draining");
        self.active_relays.fetch_add(1, Ordering::AcqRel);
        Ok(RequestAdmission(self.active_relays.clone()))
    }

    pub(crate) async fn admit_request(
        &self,
        tenant: &str,
        method: &str,
    ) -> anyhow::Result<RequestAdmission> {
        let tenants = self.tenants.lock().await;
        anyhow::ensure!(
            !self.is_draining() || allowed_while_draining(method),
            "replica is draining"
        );
        let entry = tenants
            .get(tenant)
            .ok_or_else(|| anyhow::anyhow!("tenant owner is unavailable"))?;
        entry.active_requests.fetch_add(1, Ordering::AcqRel);
        Ok(RequestAdmission(entry.active_requests.clone()))
    }

    /// Poll after removing this replica from new-session routing. Only zero
    /// remaining tenant and relay counts with no error is a successful drain receipt. Failed or
    /// unknown releases remain resident and cannot be hidden by idle eviction.
    pub async fn poll_owner_drain(&self) -> anyhow::Result<HostedOwnerDrainStatus> {
        let _operation = self.drain_operation.lock().await;
        anyhow::ensure!(self.is_draining(), "owner drain has not started");
        let candidates = self
            .tenants
            .lock()
            .await
            .iter()
            .filter(|(_, entry)| entry.active_requests.load(Ordering::Acquire) == 0)
            .map(|(tenant, entry)| (tenant.clone(), entry.server.clone()))
            .collect::<Vec<_>>();
        let mut released = 0;
        let mut failures = 0;
        for (tenant, server) in candidates {
            match server.release_idle_runtime_owner().await {
                Ok(true) => {
                    let mut tenants = self.tenants.lock().await;
                    if tenants
                        .get(&tenant)
                        .is_some_and(|entry| Arc::ptr_eq(&entry.server, &server))
                    {
                        tenants.remove(&tenant);
                        released += 1;
                    }
                }
                Ok(false) => {}
                Err(_) => failures += 1,
            }
        }
        anyhow::ensure!(
            failures == 0,
            "{failures} runtime owner releases remain unconfirmed"
        );
        Ok(HostedOwnerDrainStatus {
            remaining_tenants: self.tenants.lock().await.len(),
            remaining_relays: self.active_relays.load(Ordering::Acquire),
            released_tenants: released,
        })
    }
}

fn allowed_while_draining(method: &str) -> bool {
    matches!(
        method,
        "hosted/whoami"
            | "thread/resume"
            | "thread/resolve_approval"
            | "thread/resolve_user_input"
            | "tools/bind_executor"
            | "tools/unbind_executor"
            | "tools/resolve"
            | "turn/interrupt"
            | "turn/steer"
    ) || app_server_method_specs()
        .iter()
        .any(|spec| spec.method == method && spec.side_effect == AppServerSideEffect::ReadOnly)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::HostedRuntimeLeaseBackend;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::{
        Runtime, RuntimeConfig, RuntimeExecutionLease, fake_provider::FakeInferenceEngine,
    };

    struct Backend(bool);
    #[async_trait::async_trait]
    impl HostedRuntimeLeaseBackend for Backend {
        async fn renew(&self, _: Duration) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn release(&self) -> anyhow::Result<bool> {
            Ok(self.0)
        }
    }
    fn pool(confirmed: bool) -> HostedRuntimePool {
        HostedRuntimePool::new(
            HostedRuntimeProfile {
                data_root: std::env::temp_dir()
                    .join(format!("roder-pool-drain-{}", uuid::Uuid::new_v4())),
                idle_ttl: Duration::ZERO,
                ..Default::default()
            },
            Arc::new(move |_, _| {
                Box::pin(async move {
                    let authority = Arc::new(RuntimeExecutionLease::new(
                        Instant::now() + Duration::from_secs(30),
                    ));
                    let mut registry = ExtensionRegistryBuilder::new();
                    registry.inference_engine(Arc::new(FakeInferenceEngine));
                    let runtime = Runtime::new(registry.build()?, RuntimeConfig::default())?
                        .with_execution_lease(authority.clone());
                    let server = Arc::new(AppServer::new(Arc::new(runtime)));
                    server.supervise_runtime_lease(
                        authority,
                        Arc::new(Backend(confirmed)),
                        Duration::from_secs(30),
                    )?;
                    Ok(server)
                })
            }),
        )
    }

    #[tokio::test]
    async fn drain_waits_for_admitted_requests_and_preserves_completion_messages() {
        let pool = pool(true);
        let old = pool.lease("tenant").await.unwrap();
        let admitted = pool.admit_request("tenant", "command/exec").await.unwrap();
        pool.begin_owner_drain().await;
        assert!(pool.admit_request("tenant", "turn/start").await.is_err());
        assert!(pool.admit_request("tenant", "command/exec").await.is_err());
        assert!(pool.lease("new-tenant").await.is_err());
        assert!(
            pool.lease("tenant").await.is_ok(),
            "active owners remain reconnectable"
        );
        for method in [
            "tools/resolve",
            "thread/resolve_approval",
            "thread/resolve_user_input",
            "tools/bind_executor",
            "thread/read",
        ] {
            assert!(
                pool.admit_request("tenant", method).await.is_ok(),
                "{method}"
            );
        }
        assert_eq!(pool.poll_owner_drain().await.unwrap().remaining_tenants, 1);
        old.server.runtime.ensure_execution_authority().unwrap();
        drop(admitted);
        assert_eq!(
            pool.poll_owner_drain().await.unwrap(),
            HostedOwnerDrainStatus {
                remaining_tenants: 0,
                remaining_relays: 0,
                released_tenants: 1
            }
        );
        assert!(old.server.runtime.ensure_execution_authority().is_err());
        assert_eq!(pool.poll_owner_drain().await.unwrap().remaining_tenants, 0);
        assert!(pool.lease("tenant").await.is_err());
        pool.resume_owner_admission().await;
        let fresh = pool.lease("tenant").await.unwrap();
        assert!(!Arc::ptr_eq(&old.server, &fresh.server));
        fresh.server.runtime.ensure_execution_authority().unwrap();
    }

    #[tokio::test]
    async fn unconfirmed_release_cannot_disappear_through_reconnect_or_eviction() {
        let pool = pool(false);
        drop(pool.lease("tenant").await.unwrap());
        pool.begin_owner_drain().await;
        assert!(pool.poll_owner_drain().await.is_err());
        assert!(pool.lease("tenant").await.is_err());
        assert!(pool.evict_idle().await.is_empty());
        assert_eq!(pool.len().await, 1);
        assert!(pool.poll_owner_drain().await.is_err());
    }
    #[tokio::test]
    async fn forwarded_connections_prevent_a_false_complete_drain() {
        let pool = pool(true);
        let relay = pool.admit_owner_relay().await.unwrap();
        pool.begin_owner_drain().await;
        assert!(pool.admit_owner_relay().await.is_err());
        assert_eq!(pool.poll_owner_drain().await.unwrap().remaining_relays, 1);
        drop(relay);
        assert_eq!(pool.poll_owner_drain().await.unwrap().remaining_relays, 0);
    }
}
