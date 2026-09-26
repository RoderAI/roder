use super::*;
impl Runtime {
    /// Manual compaction owns this task's admission until its boundary is
    /// committed. Other tasks retain independent admission and keep running.
    pub(crate) async fn thread_admission(
        &self,
        thread_id: &ThreadId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let gate = self
            .thread_admission_gates
            .lock()
            .await
            .entry(thread_id.clone())
            .or_default()
            .clone();
        gate.lock_owned().await
    }
}
