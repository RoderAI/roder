use super::*;

impl Runtime {
    /// A host must never observe a request (or resolution acknowledgement)
    /// before the configured store can recover its execution identity.
    pub(crate) async fn emit_external_execution(&self, event: RoderEvent) -> anyhow::Result<()> {
        self.ensure_execution_authority()?;
        let prepared = self.bus.prepare(event);
        let envelope = prepared.envelope();
        if let (Some(store), Some(thread_id)) = (&self.thread_store, envelope.thread_id.as_ref())
            && should_persist_thread_event(thread_id)
        {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                store.append_event(thread_id, envelope),
            )
            .await
            .map_err(|_| anyhow::anyhow!("external execution receipt persistence timed out"))??;
        }
        self.ensure_execution_authority()?;
        let envelope = envelope.clone();
        prepared.publish();
        self.dispatch_event_sinks(&envelope).await;
        Ok(())
    }

    pub(super) async fn dispatch_event_sinks(&self, envelope: &EventEnvelope) {
        let dispatcher = self
            .event_sink_dispatcher
            .get_or_init(|| async {
                crate::event_sink_dispatch::EventSinkDispatcher::start(
                    &self.registry.event_sinks,
                    self.bus.clone(),
                )
            })
            .await;
        if !dispatcher.is_empty() {
            dispatcher.dispatch(envelope, &self.bus);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use tokio::sync::Notify;

    struct GatedStore {
        entered: Notify,
        release: Notify,
        fail: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ThreadStore for GatedStore {
        fn id(&self) -> String {
            "gated".into()
        }
        async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
            Ok(metadata)
        }
        async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
            Ok(vec![])
        }
        async fn load_thread(&self, _: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
            Ok(None)
        }
        async fn append_event(&self, _: &ThreadId, _: &EventEnvelope) -> anyhow::Result<()> {
            self.entered.notify_one();
            self.release.notified().await;
            anyhow::ensure!(
                !self.fail.load(Ordering::SeqCst),
                "injected persistence failure"
            );
            Ok(())
        }
    }

    #[tokio::test]
    async fn external_receipts_are_not_delivered_until_persistence_succeeds() {
        for resolved in [false, true] {
            for fail in [false, true] {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
                let mut runtime =
                    Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
                let store = Arc::new(GatedStore {
                    entered: Notify::new(),
                    release: Notify::new(),
                    fail: AtomicBool::new(fail),
                });
                runtime.thread_store = Some(store.clone());
                let runtime = Arc::new(runtime);
                let mut notifications = runtime.bus.subscribe();
                let event = if resolved {
                    RoderEvent::ExternalToolCallResolved(ExternalToolCallResolved {
                        thread_id: "thread-1".into(),
                        turn_id: "turn-1".into(),
                        request_id: "request-1".into(),
                        tool_id: "tool-1".into(),
                        tool_name: "save".into(),
                        outcome: ExternalToolCallOutcome::Resolved,
                        is_error: false,
                        timestamp: OffsetDateTime::now_utc(),
                    })
                } else {
                    RoderEvent::ExternalToolCallRequested(ExternalToolCallRequested {
                        thread_id: "thread-1".into(),
                        turn_id: "turn-1".into(),
                        request_id: "request-1".into(),
                        tool_id: "tool-1".into(),
                        tool_name: "save".into(),
                        arguments: serde_json::json!({}),
                        timestamp: OffsetDateTime::now_utc(),
                    })
                };
                let task =
                    tokio::spawn(async move { runtime.emit_external_execution(event).await });
                tokio::time::timeout(std::time::Duration::from_secs(5), store.entered.notified())
                    .await
                    .unwrap();
                assert!(
                    notifications.try_recv().is_err(),
                    "delivery before persistence"
                );
                store.release.notify_one();
                assert_eq!(task.await.unwrap().is_err(), fail);
                assert_eq!(notifications.try_recv().is_ok(), !fail);
            }
        }
    }

    #[tokio::test]
    async fn failed_resolution_receipt_keeps_the_original_waiter_available_for_retry() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let mut runtime = Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
        let store = Arc::new(GatedStore {
            entered: Notify::new(),
            release: Notify::new(),
            fail: AtomicBool::new(true),
        });
        runtime.thread_store = Some(store.clone());
        let runtime = Arc::new(runtime);
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        runtime.pending_external_tool_calls.lock().await.insert(
            "request".into(),
            PendingExternalToolCall {
                thread_id: "thread-1".into(),
                turn_id: "turn".into(),
                tool_id: "tool".into(),
                tool_name: "save".into(),
                tx: sender,
            },
        );
        for fail in [true, false] {
            store.fail.store(fail, Ordering::SeqCst);
            let runtime = runtime.clone();
            let resolution = tokio::spawn(async move {
                runtime
                    .resolve_external_tool_call(
                        "request",
                        ExternalToolResolution {
                            output: "saved".into(),
                            is_error: false,
                        },
                    )
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), store.entered.notified())
                .await
                .unwrap();
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            store.release.notify_one();
            let outcome = resolution.await.unwrap();
            if fail {
                assert!(outcome.is_err());
            } else {
                assert!(outcome.unwrap());
            }
        }
        assert_eq!(receiver.await.unwrap().output, "saved");
        assert!(runtime.pending_external_executions().await.is_empty());
    }
}
