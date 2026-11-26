use super::*;
use roder_api::thread::{ThreadHistoryPage, ThreadHistoryReadOptions};

impl Runtime {
    /// Read a bounded history page without hydrating a full thread or its live execution state.
    pub async fn read_thread_history(
        &self,
        thread_id: &ThreadId,
        options: ThreadHistoryReadOptions,
    ) -> anyhow::Result<Option<ThreadHistoryPage>> {
        let store = self
            .thread_store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("thread history requires a thread store"))?;
        store.read_history_page(thread_id, options).await
    }

    /// Resume a paginated thread without loading its transcript.
    pub async fn resume_thread_history(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<TurnLifecycleSnapshot> {
        let admission = self.thread_admission(thread_id).await;
        let mut lifecycle = self.turn_lifecycle_snapshot(thread_id).await?;
        if lifecycle
            .records
            .iter()
            .any(|record| record.state.requires_recovery())
        {
            let live_turn_ids = self
                .active_turns
                .read()
                .await
                .keys()
                .cloned()
                .chain(self.turn_drains.read().await.keys().cloned())
                .collect::<HashSet<_>>();
            let mut reconciled = false;
            for record in lifecycle.records.iter().filter(|record| {
                record.state.requires_recovery() && !live_turn_ids.contains(&record.turn_id)
            }) {
                self.lifecycle_restart_reconciliations
                    .fetch_add(1, Ordering::AcqRel);
                self.record_turn_lifecycle(
                    record.thread_id.clone(),
                    record.turn_id.clone(),
                    TurnLifecycleState::RecoveryNeeded,
                    TurnCleanupState::Unknown,
                    Some(TurnLifecycleReason::RuntimeRestart),
                )
                .await;
                reconciled = true;
            }
            if reconciled {
                lifecycle = self.turn_lifecycle_snapshot(thread_id).await?;
            }
        }
        // Local resume hooks can reenter the runtime.
        drop(admission);
        self.emit_thread_loaded(thread_id).await;
        Ok(lifecycle)
    }

    /// Preserve session-start behavior for a stored thread used without an explicit resume.
    pub async fn prepare_stored_thread_for_turn(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let started = self.session_hook_started.read().await.contains(thread_id);
        if !started {
            self.resume_thread_history(thread_id).await?;
        }
        Ok(())
    }

    pub(super) async fn emit_thread_loaded(&self, thread_id: &ThreadId) {
        self.emit(RoderEvent::ThreadLoaded(ThreadLoaded {
            thread_id: thread_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::extension_state::ExtensionStateRecord;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct PageOnlyStore {
        request: Mutex<Option<(ThreadId, ThreadHistoryReadOptions)>>,
        lifecycle: Mutex<Vec<ExtensionStateRecord>>,
    }

    #[async_trait::async_trait]
    impl ThreadStore for PageOnlyStore {
        fn id(&self) -> String {
            "page-only".into()
        }

        async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
            Ok(metadata)
        }

        async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
            Ok(Vec::new())
        }

        async fn load_thread(&self, _: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
            panic!("a history page must never load the complete thread")
        }

        async fn read_history_page(
            &self,
            thread_id: &ThreadId,
            options: ThreadHistoryReadOptions,
        ) -> anyhow::Result<Option<ThreadHistoryPage>> {
            *self.request.lock().unwrap() = Some((thread_id.clone(), options));
            Ok(None)
        }

        async fn load_extension_states(
            &self,
            _: &ThreadId,
        ) -> anyhow::Result<Vec<ExtensionStateRecord>> {
            Ok(self.lifecycle.lock().unwrap().clone())
        }

        async fn append_extension_state(
            &self,
            _: &ThreadId,
            record: &ExtensionStateRecord,
        ) -> anyhow::Result<()> {
            self.lifecycle.lock().unwrap().push(record.clone());
            Ok(())
        }

        async fn append_event(&self, _: &ThreadId, _: &EventEnvelope) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn history_read_delegates_bounds_without_loading_or_hydrating_the_thread() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let mut runtime = Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
        let store = Arc::new(PageOnlyStore::default());
        runtime.thread_store = Some(store.clone());
        let page = runtime
            .read_thread_history(
                &"thread-1".into(),
                ThreadHistoryReadOptions {
                    before: Some("cursor".into()),
                    limit: 120,
                    max_bytes: 2048,
                },
            )
            .await
            .unwrap();
        assert!(page.is_none());
        {
            let request = store.request.lock().unwrap();
            let (thread_id, options) = request.as_ref().unwrap();
            assert_eq!(thread_id, "thread-1");
            assert_eq!(options.before.as_deref(), Some("cursor"));
            assert_eq!(options.limit, 120);
            assert_eq!(options.max_bytes, 2048);
        }
        assert!(runtime.active_turns.read().await.is_empty());
    }

    #[tokio::test]
    async fn resume_waits_for_an_in_progress_admission_before_recovering_lifecycle() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let mut runtime = Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
        let store = Arc::new(PageOnlyStore::default());
        runtime.thread_store = Some(store.clone());
        let runtime = Arc::new(runtime);
        let thread_id = "admitting-thread".to_string();
        let admission = runtime.thread_admission(&thread_id).await;
        let running = TurnLifecycleRecord::new(
            thread_id.clone(),
            "turn".to_string(),
            TurnLifecycleState::Running,
            TurnCleanupState::NotRequested,
            None,
            OffsetDateTime::UNIX_EPOCH,
        );
        store
            .append_extension_state(&thread_id, &running.extension_state().unwrap())
            .await
            .unwrap();

        let mut resume = tokio::spawn({
            let runtime = runtime.clone();
            let thread_id = thread_id.clone();
            async move { runtime.resume_thread_history(&thread_id).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut resume)
                .await
                .is_err(),
            "resume must not reconcile the half-admitted turn"
        );

        let completed = TurnLifecycleRecord::new(
            thread_id.clone(),
            "turn".to_string(),
            TurnLifecycleState::Completed,
            TurnCleanupState::Completed,
            None,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
        );
        store
            .append_extension_state(&thread_id, &completed.extension_state().unwrap())
            .await
            .unwrap();
        drop(admission);

        let snapshot = tokio::time::timeout(Duration::from_secs(2), resume)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.records[0].state, TurnLifecycleState::Completed);
        assert_eq!(store.lifecycle.lock().unwrap().len(), 2);
    }
}
