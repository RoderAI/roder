//! Planned owner handoff is separate from cancellation-based shutdown.
use super::Runtime;
use std::sync::atomic::Ordering;

impl Runtime {
    /// Permanently seal an owned runtime only after all admitted turns, tool
    /// futures, and terminal cleanup have finished. Busy runtimes keep working;
    /// the host must stop new inbound work while polling this operation.
    ///
    /// True proves local quiescence, including after lease expiry or revocation.
    /// The host must separately confirm loss of the durable generation before
    /// reporting a completed handoff. False means work remains. Persistence
    /// failures still require reconciliation and cannot be reported as drained.
    pub async fn seal_idle_owner(&self) -> anyhow::Result<bool> {
        let _admission = self.turn_admission.lock().await;
        let lease = self
            .execution_lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner handoff requires an execution lease"))?;
        if !self.active_turns.read().await.is_empty() || !self.turn_drains.read().await.is_empty() {
            return Ok(false);
        }
        anyhow::ensure!(
            self.lifecycle_persistence_failures.load(Ordering::Acquire) == 0,
            "owner handoff requires reconciliation of failed lifecycle persistence"
        );
        if !lease.seal_if_idle()? {
            return Ok(false);
        }
        self.accepting_turns.store(false, Ordering::Release);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeConfig, RuntimeExecutionLease, fake_provider::FakeInferenceEngine};
    use roder_api::extension::ExtensionRegistryBuilder;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    #[tokio::test]
    async fn expired_idle_owner_can_seal_but_persistence_failure_blocks_proof() {
        let lease = Arc::new(RuntimeExecutionLease::new(Instant::now()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default())
            .unwrap()
            .with_execution_lease(lease.clone());
        runtime
            .lifecycle_persistence_failures
            .store(1, Ordering::Release);
        assert!(runtime.seal_idle_owner().await.is_err());
        runtime
            .lifecycle_persistence_failures
            .store(0, Ordering::Release);
        assert!(runtime.seal_idle_owner().await.unwrap());
        assert!(runtime.seal_idle_owner().await.unwrap());
        assert!(runtime.ensure_execution_authority().is_err());
        assert!(
            lease
                .renew(Instant::now() + Duration::from_secs(30))
                .is_err()
        );
    }
}
