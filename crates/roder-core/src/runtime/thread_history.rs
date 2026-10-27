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
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use std::sync::Mutex;

    #[derive(Default)]
    struct PageOnlyStore {
        request: Mutex<Option<(ThreadId, ThreadHistoryReadOptions)>>,
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
}
