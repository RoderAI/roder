use super::*;

impl Runtime {
    /// Save the admitted prompt before its worker can be interrupted. Reserve the
    /// start and prompt sequence together so live clients see start first.
    pub(super) async fn prepare_turn_start(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        runtime_profile: RuntimeProfile,
    ) -> anyhow::Result<[crate::bus::PreparedEvent; 3]> {
        // Admission just allocated this turn id, so its first item is always
        // at index zero. A cold cache can hydrate when the worker needs it.
        let item_index = 0;
        let record = TurnLifecycleRecord::new(
            req.thread_id.clone(),
            turn_id.clone(),
            TurnLifecycleState::Running,
            TurnCleanupState::NotRequested,
            None,
            OffsetDateTime::now_utc(),
        );
        // Restart reconciliation can now identify an attempt even when a crash
        // cuts off one of the subsequent transcript writes.
        let _ = self.persist_turn_lifecycle_record(&record).await;
        let lifecycle = self.bus.prepare(RoderEvent::TurnLifecycleUpdated(record));
        let started = self.bus.prepare(RoderEvent::TurnStarted(TurnStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            runtime_profile,
            timestamp: OffsetDateTime::now_utc(),
        }));
        let prompt = self.bus.prepare(turn_item_event(
            &req.thread_id,
            turn_id,
            &TranscriptItem::UserMessage(UserMessage::with_images(
                req.message.clone(),
                req.images.clone(),
            )),
            item_index,
        ));
        let store = self
            .thread_store
            .as_ref()
            .filter(|_| should_persist_thread_event(&req.thread_id));
        let result = async {
            if let Some(store) = store {
                let _ = store
                    .append_event(&req.thread_id, lifecycle.envelope())
                    .await;
                store
                    .append_event(&req.thread_id, started.envelope())
                    .await
                    .context("persist turn start")?;
                store
                    .append_event(&req.thread_id, prompt.envelope())
                    .await
                    .context("persist accepted turn input")?;
            }
            self.ensure_execution_authority()
        }
        .await;
        if let Err(error) = result {
            // A store can fail after appending an event. Close any partial
            // history and release the reserved bus slots without announcing a start.
            drop((lifecycle, started, prompt));
            self.record_rejected_turn(&req.thread_id, turn_id).await;
            return Err(error);
        }
        let mut cache = self.thread_item_cache.lock().await;
        if store.is_none() || cache.contains_thread(&req.thread_id) {
            cache.remember_transcript_item_index(&req.thread_id, turn_id, item_index);
        }
        Ok([lifecycle, started, prompt])
    }

    pub(super) async fn record_rejected_turn(&self, thread_id: &ThreadId, turn_id: &TurnId) {
        let record = TurnLifecycleRecord::new(
            thread_id.clone(),
            turn_id.clone(),
            TurnLifecycleState::Failed,
            TurnCleanupState::NotRequested,
            Some(TurnLifecycleReason::RuntimeFailure),
            OffsetDateTime::now_utc(),
        );
        let _ = self.persist_turn_lifecycle_record(&record).await;
        if let Some(store) = &self.thread_store
            && should_persist_thread_event(thread_id)
        {
            let failed = self.bus.prepare(RoderEvent::TurnFailed(TurnFailed {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                error: "turn admission could not be persisted".to_string(),
                error_kind: None,
                usage: None,
                timestamp: record.timestamp,
            }));
            let _ = store.append_event(thread_id, failed.envelope()).await;
        }
    }
}
