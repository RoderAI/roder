use super::*;

impl Runtime {
    /// Cancel and drain one thread, then release its live caches and runner.
    /// Persisted conversation data remains available for subsequent turns.
    /// Callers should apply a deadline; failed cleanup retains resources for retry.
    pub async fn unload_thread(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let _admission = self.thread_admission(thread_id).await;
        if self.goals.active_goal(thread_id).await?.is_some() {
            self.thread_goal_set(
                thread_id,
                roder_api::goals::ThreadGoalPatch {
                    status: Some(roder_api::goals::ThreadGoalStatus::Paused),
                    ..Default::default()
                },
            )
            .await?;
        }
        let active = self
            .active_turns
            .read()
            .await
            .iter()
            .filter(|(_, handle)| &handle.thread_id == thread_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for turn_id in active {
            self.interrupt_turn(thread_id.clone(), turn_id).await?;
        }
        loop {
            let changed = self.active_turns_changed.notified();
            let active = self
                .active_turns
                .read()
                .await
                .values()
                .any(|handle| &handle.thread_id == thread_id);
            let draining = self
                .turn_drains
                .read()
                .await
                .values()
                .any(|drain| &drain.thread_id == thread_id);
            if !active && !draining {
                break;
            }
            changed.await;
        }
        let lifecycle = self.turn_lifecycle_snapshot(thread_id).await?;
        anyhow::ensure!(
            !lifecycle
                .records
                .iter()
                .any(|record| record.ownership == TurnCleanupOwnership::ProviderCleanupPending),
            "provider cleanup is not confirmed for thread {thread_id}"
        );
        let slot = self.runner_sessions.lock().await.get(thread_id).cloned();
        if let Some(slot) = slot {
            loop {
                let initialized = slot.initialized.notified();
                let state = slot.state.lock().await;
                match &*state {
                    RunnerSessionSlotState::Initializing => {
                        drop(state);
                        initialized.await;
                    }
                    RunnerSessionSlotState::Ready(cached) => {
                        let session = cached.session.clone();
                        drop(state);
                        session.close().await?;
                        break;
                    }
                    _ => break,
                }
            }
            self.evict_runner_session(thread_id).await;
        }
        self.thread_item_cache.lock().await.remove_thread(thread_id);
        self.thread_policy_modes.write().await.remove(thread_id);
        Ok(())
    }
}
