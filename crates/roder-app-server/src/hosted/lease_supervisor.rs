//! Bounded renewal of one immutable durable runtime-owner generation.
use crate::AppServer;
use roder_core::RuntimeExecutionLease;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

/// The host binds this backend to an exact tenant, process and generation.
#[async_trait::async_trait]
pub trait HostedRuntimeLeaseBackend: Send + Sync + 'static {
    /// Return true only after confirming that exact generation was renewed for
    /// at least `ttl`. False means lost authority; errors are unknown outcomes.
    async fn renew(&self, ttl: Duration) -> anyhow::Result<bool>;
    /// Confirm this exact generation no longer has durable authority, after
    /// local execution has been sealed. Repeated confirmation must be safe.
    async fn release(&self) -> anyhow::Result<bool>;
}

pub(crate) struct HostedRuntimeLeaseSupervisor {
    authority: Arc<RuntimeExecutionLease>,
    task: tokio::task::JoinHandle<()>,
    backend: Arc<dyn HostedRuntimeLeaseBackend>,
    ttl: Duration,
    sealed: bool,
    released: bool,
}

impl Drop for HostedRuntimeLeaseSupervisor {
    fn drop(&mut self) {
        // Runtime clones may outlive the app-server. They must not retain
        // execution authority after their renewal supervisor is gone.
        self.authority.revoke();
        self.task.abort();
    }
}

struct RevokeOnExit(Arc<RuntimeExecutionLease>);
impl Drop for RevokeOnExit {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

impl AppServer {
    /// Poll after stopping new inbound work. Busy runtimes are untouched. A
    /// successful result proves local quiescence and durable lease release;
    /// errors are not safe-to-terminate receipts and cannot revive this runtime.
    pub async fn release_idle_runtime_owner(&self) -> anyhow::Result<bool> {
        let _release = self.runtime_owner_release.lock().await;
        let (backend, timeout, sealed) = {
            let installed = self
                .runtime_lease_supervisor
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime lease supervisor unavailable"))?;
            let supervisor = installed
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("runtime lease supervisor is not installed"))?;
            if supervisor.released {
                return Ok(true);
            }
            (
                supervisor.backend.clone(),
                supervisor.ttl / 3,
                supervisor.sealed,
            )
        };
        if !sealed && !self.runtime.seal_idle_owner().await? {
            return Ok(false);
        }
        // Retain confirmation state across timeouts, errors and cancellation.
        // The sealed runtime cannot execute or renew while confirmation retries.
        {
            let mut installed = self
                .runtime_lease_supervisor
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime lease supervisor unavailable"))?;
            let supervisor = installed
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("runtime lease supervisor is not installed"))?;
            supervisor.authority.revoke();
            supervisor.task.abort();
            supervisor.sealed = true;
        }
        let released = tokio::time::timeout(timeout, backend.release()).await??;
        anyhow::ensure!(released, "runtime owner release was not confirmed");
        self.runtime_lease_supervisor
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime lease supervisor unavailable"))?
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("runtime lease supervisor is not installed"))?
            .released = true;
        Ok(true)
    }

    /// Install before publishing this server. The same guard must already be
    /// bound to its runtime; duplicate installation cannot replace ownership.
    pub fn supervise_runtime_lease(
        self: &Arc<Self>,
        authority: Arc<RuntimeExecutionLease>,
        backend: Arc<dyn HostedRuntimeLeaseBackend>,
        ttl: Duration,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            ttl >= Duration::from_secs(1) && ttl <= Duration::from_secs(300),
            "runtime owner TTL must be between 1 and 300 seconds"
        );
        anyhow::ensure!(
            self.runtime.uses_execution_lease(&authority),
            "supervisor must use the runtime's execution lease"
        );
        authority.require_live()?;
        let mut installed = self
            .runtime_lease_supervisor
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime lease supervisor unavailable"))?;
        anyhow::ensure!(
            installed.is_none(),
            "runtime lease supervisor already installed"
        );
        let server = Arc::downgrade(self);
        let task_authority = authority.clone();
        let task = tokio::runtime::Handle::try_current()?.spawn(supervise(
            server,
            task_authority,
            backend.clone(),
            ttl,
        ));
        *installed = Some(HostedRuntimeLeaseSupervisor {
            authority,
            task,
            backend,
            ttl,
            sealed: false,
            released: false,
        });
        Ok(())
    }
}

async fn supervise(
    server: Weak<AppServer>,
    authority: Arc<RuntimeExecutionLease>,
    backend: Arc<dyn HostedRuntimeLeaseBackend>,
    ttl: Duration,
) {
    // Covers panics and task cancellation as well as ordinary renewal failure.
    let _revoke_on_exit = RevokeOnExit(authority.clone());
    let cadence = ttl / 3;
    let mut interval = tokio::time::interval(cadence);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        if authority.require_live().is_err() {
            break;
        }
        let requested_at = Instant::now();
        match tokio::time::timeout(cadence, backend.renew(ttl)).await {
            Ok(Ok(true)) if authority.renew(requested_at + ttl).is_ok() => {}
            _ => break,
        }
    }
    authority.revoke();
    if let Some(server) = server.upgrade() {
        // Unknown DB outcomes cannot justify more actions. Cleanup is bounded;
        // new owners must reconcile any external operation already in flight.
        let timeout = Duration::from_secs(5);
        let _ = tokio::time::timeout(timeout, server.drain_runtime(timeout)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::{Runtime, RuntimeConfig, fake_provider::FakeInferenceEngine};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Backend {
        renewals: AtomicUsize,
        valid: AtomicBool,
        hang: bool,
    }
    #[async_trait::async_trait]
    impl HostedRuntimeLeaseBackend for Backend {
        async fn release(&self) -> anyhow::Result<bool> {
            Ok(self.valid.swap(false, Ordering::SeqCst))
        }
        async fn renew(&self, _ttl: Duration) -> anyhow::Result<bool> {
            self.renewals.fetch_add(1, Ordering::SeqCst);
            if self.hang {
                return std::future::pending().await;
            }
            Ok(self.valid.load(Ordering::SeqCst))
        }
    }
    fn fixture() -> (Arc<AppServer>, Arc<RuntimeExecutionLease>) {
        let authority = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(1),
        ));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default())
            .unwrap()
            .with_execution_lease(authority.clone());
        (Arc::new(AppServer::new(Arc::new(runtime))), authority)
    }
    fn backend(hang: bool) -> Arc<Backend> {
        Arc::new(Backend {
            renewals: AtomicUsize::new(0),
            valid: AtomicBool::new(true),
            hang,
        })
    }
    async fn wait_for_loss(authority: &RuntimeExecutionLease) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while authority.require_live().is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn renews_live_authority_then_revokes_when_the_generation_is_lost() {
        let (server, authority) = fixture();
        let backend = backend(false);
        server
            .supervise_runtime_lease(authority.clone(), backend.clone(), Duration::from_secs(1))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(backend.renewals.load(Ordering::SeqCst) >= 2);
        authority.require_live().unwrap();
        backend.valid.store(false, Ordering::SeqCst);
        wait_for_loss(&authority).await;
        assert!(
            authority
                .renew(Instant::now() + Duration::from_secs(30))
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_unresponsive_backend_fails_closed() {
        let (server, authority) = fixture();
        server
            .supervise_runtime_lease(authority.clone(), backend(true), Duration::from_secs(1))
            .unwrap();
        wait_for_loss(&authority).await;
    }

    #[tokio::test]
    async fn dropping_the_server_revokes_surviving_runtime_clones() {
        let (server, authority) = fixture();
        let runtime = server.runtime.clone();
        server
            .supervise_runtime_lease(authority, backend(false), Duration::from_secs(1))
            .unwrap();
        drop(server);
        assert!(runtime.ensure_execution_authority().is_err());
    }

    #[tokio::test]
    async fn mismatched_or_duplicate_supervisors_cannot_change_authority() {
        let (server, authority) = fixture();
        let different = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(30),
        ));
        assert!(
            server
                .supervise_runtime_lease(different, backend(false), Duration::from_secs(1))
                .is_err()
        );
        server
            .supervise_runtime_lease(authority.clone(), backend(false), Duration::from_secs(1))
            .unwrap();
        assert!(
            server
                .supervise_runtime_lease(authority.clone(), backend(true), Duration::from_secs(1))
                .is_err()
        );
        authority.require_live().unwrap();
    }
    #[tokio::test]
    async fn idle_handoff_revokes_authority_and_confirms_release() {
        let (server, authority) = fixture();
        let backend = backend(false);
        server
            .supervise_runtime_lease(authority.clone(), backend.clone(), Duration::from_secs(1))
            .unwrap();
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert!(!backend.valid.load(Ordering::SeqCst));
        assert!(authority.require_live().is_err());
    }

    #[tokio::test]
    async fn unconfirmed_release_cannot_restore_local_execution() {
        let (server, authority) = fixture();
        let backend = backend(false);
        server
            .supervise_runtime_lease(authority.clone(), backend.clone(), Duration::from_secs(1))
            .unwrap();
        backend.valid.store(false, Ordering::SeqCst);
        assert!(server.release_idle_runtime_owner().await.is_err());
        assert!(authority.require_live().is_err());
        backend.valid.store(true, Ordering::SeqCst);
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert!(authority.require_live().is_err());
        assert!(
            authority
                .renew(Instant::now() + Duration::from_secs(30))
                .is_err()
        );
    }

    struct DelayedRelease {
        attempts: AtomicUsize,
        started: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl HostedRuntimeLeaseBackend for DelayedRelease {
        async fn release(&self) -> anyhow::Result<bool> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                self.started.notify_one();
                return std::future::pending().await;
            }
            Ok(true)
        }
        async fn renew(&self, _: Duration) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn cancelled_release_retains_confirmation_state_and_never_renews() {
        let (server, authority) = fixture();
        let backend = Arc::new(DelayedRelease {
            attempts: AtomicUsize::new(0),
            started: tokio::sync::Notify::new(),
        });
        server
            .supervise_runtime_lease(authority.clone(), backend.clone(), Duration::from_secs(1))
            .unwrap();
        let releasing = server.clone();
        let task = tokio::spawn(async move { releasing.release_idle_runtime_owner().await });
        tokio::time::timeout(Duration::from_secs(3), backend.started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(authority.require_live().is_err());
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 2);
        assert!(authority.require_live().is_err());
    }

    #[tokio::test]
    async fn timed_out_release_can_be_confirmed_on_a_later_poll() {
        let (server, authority) = fixture();
        let backend = Arc::new(DelayedRelease {
            attempts: AtomicUsize::new(0),
            started: tokio::sync::Notify::new(),
        });
        server
            .supervise_runtime_lease(authority.clone(), backend.clone(), Duration::from_secs(1))
            .unwrap();
        assert!(server.release_idle_runtime_owner().await.is_err());
        assert!(authority.require_live().is_err());
        assert!(server.release_idle_runtime_owner().await.unwrap());
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 2);
    }
}
