//! Planned owner handoff is separate from cancellation-based shutdown.
use super::Runtime;
use std::sync::atomic::Ordering;

impl Runtime {
    /// Permanently seal an owned runtime only after all admitted turns, tool
    /// futures, and terminal cleanup have finished. Busy runtimes keep working;
    /// the host must stop new inbound work while polling this operation.
    ///
    /// True permits the host to release its durable generation. False means
    /// work remains. Errors (including persistence failure or lost ownership)
    /// require recovery; they are not evidence of a successful handoff.
    pub async fn seal_idle_owner(&self) -> anyhow::Result<bool> {
        let _admission = self.turn_admission.lock().await;
        let lease = self
            .execution_lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner handoff requires an execution lease"))?;
        lease.require_live()?;
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
