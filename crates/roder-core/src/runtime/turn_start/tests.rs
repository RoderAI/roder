use super::*;
use crate::RuntimeExecutionLease;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_ext_jsonl_thread_store::store::JsonlThreadStore;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

struct PromptStore {
    store: JsonlThreadStore,
    writing_prompt: Notify,
    release_prompt: Notify,
    interrupted: Notify,
    reached_context_assembly: Notify,
    fail_prompt: bool,
    full_reads: AtomicUsize,
}

#[async_trait::async_trait]
impl ThreadStore for PromptStore {
    fn id(&self) -> String {
        "test-prompt-store".into()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.store.create_thread(metadata).await
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        self.store.list_threads().await
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        self.full_reads.fetch_add(1, Ordering::Relaxed);
        self.store.load_thread(thread_id).await
    }

    async fn load_extension_states(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Vec<roder_api::extension_state::ExtensionStateRecord>> {
        self.store.load_extension_states(thread_id).await
    }

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &roder_api::extension_state::ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        self.store.append_extension_state(thread_id, record).await
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        event: &EventEnvelope,
    ) -> anyhow::Result<()> {
        if matches!(
            &event.event,
            RoderEvent::TranscriptItemAppended(item)
                if matches!(item.item, Some(TranscriptItem::UserMessage(_)))
        ) {
            self.writing_prompt.notify_one();
            self.release_prompt.notified().await;
            anyhow::ensure!(!self.fail_prompt, "injected prompt write failure");
        }
        // Hold the worker before its first inference or tool call so the test
        // can interrupt it immediately after admission.
        if matches!(&event.event, RoderEvent::ContextAssemblyStarted(_)) {
            self.reached_context_assembly.notify_one();
            std::future::pending::<()>().await;
        }
        self.store.append_event(thread_id, event).await?;
        if matches!(&event.event, RoderEvent::TurnInterrupted(_)) {
            self.interrupted.notify_one();
        }
        Ok(())
    }
}

#[tokio::test]
async fn admission_waits_for_the_prompt_and_preserves_it_across_interrupt() {
    for (fail_prompt, revoke_lease) in [(false, false), (true, false), (false, true)] {
        let directory = tempfile::tempdir().unwrap();
        let hooks_dir = directory.path().join(".roder");
        std::fs::create_dir(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("hooks.json"), r#"{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"echo hook-ran >> hook-marker"}]}]}}"#).unwrap();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let lease = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(60),
        ));
        let mut runtime = Runtime::new(builder.build().unwrap(), Default::default())
            .unwrap()
            .with_execution_lease(lease.clone());
        let store = Arc::new(PromptStore {
            store: JsonlThreadStore {
                base_path: directory.path().join("threads"),
            },
            writing_prompt: Notify::new(),
            release_prompt: Notify::new(),
            interrupted: Notify::new(),
            reached_context_assembly: Notify::new(),
            fail_prompt,
            full_reads: AtomicUsize::new(0),
        });
        runtime.thread_store = Some(store.clone());
        let runtime = Arc::new(runtime);
        let thread_id = runtime.create_thread(None).await.unwrap().thread_id;
        let request = StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "persist me".into(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.path().display().to_string(),
            instructions: InstructionBundle::default(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        };
        let before = store.full_reads.load(Ordering::Relaxed);
        let mut events = runtime.subscribe_events();
        let mut admission = Box::pin(runtime.start_turn(request));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::select! {
                result = &mut admission => panic!("acknowledged before persisting prompt: {result:?}"),
                _ = store.writing_prompt.notified() => {}
            }
            assert!(events.try_recv().is_err());
            assert_eq!(store.full_reads.load(Ordering::Relaxed), before);
            assert!(runtime.active_turn_for_thread(&thread_id).await.is_none());
            let saved_before_ack = store.store.load_thread(&thread_id).await.unwrap().unwrap();
            let lifecycle = turn_lifecycle_snapshot(&saved_before_ack.extension_states);
            assert!(matches!(lifecycle.records.as_slice(), [record] if record.state == TurnLifecycleState::Running));
            let mut resumed = Box::pin(runtime.resume_thread_history(&thread_id));
            assert!(tokio::time::timeout(Duration::from_millis(20), &mut resumed).await.is_err());
            if revoke_lease {
                lease.revoke();
            }
            store.release_prompt.notify_one();
            let result = admission.await;
            assert_eq!(store.full_reads.load(Ordering::Relaxed), before);
            if fail_prompt || revoke_lease {
                let expected = if fail_prompt {
                    "injected prompt write failure"
                } else {
                    "runtime execution lease expired or revoked"
                };
                assert!(format!("{:#}", result.unwrap_err()).contains(expected));
                assert!(runtime.active_turn_for_thread(&thread_id).await.is_none());
                assert!(events.try_recv().is_err());
                let saved = store.store.load_thread(&thread_id).await.unwrap().unwrap();
                assert!(saved.turns.iter().all(|turn| turn.completed_at.is_some()));
                assert!(!directory.path().join("hook-marker").exists());
            } else {
                let turn_id = result.unwrap();
                assert!(matches!(events.try_recv().unwrap().event, RoderEvent::TurnLifecycleUpdated(record) if record.state == TurnLifecycleState::Running));
                assert!(matches!(events.try_recv().unwrap().event, RoderEvent::TurnStarted(_)));
                assert!(matches!(events.try_recv().unwrap().event, RoderEvent::TranscriptItemAppended(_)));
                let observed = resumed.await.unwrap();
                assert!(matches!(observed.records.as_slice(), [record] if record.state == TurnLifecycleState::Running));
                store.reached_context_assembly.notified().await;
                if cfg!(unix) {
                    let marker = std::fs::read_to_string(directory.path().join("hook-marker")).unwrap();
                    assert_eq!(marker.trim(), "hook-ran");
                }
                runtime.interrupt_turn(thread_id.clone(), turn_id.clone()).await.unwrap();
                store.interrupted.notified().await;
                let saved = store.store.load_thread(&thread_id).await.unwrap().unwrap();
                let turn = saved.turns.iter().find(|turn| turn.turn_id == turn_id).unwrap();
                assert!(matches!(turn.items.as_slice(), [TranscriptItem::UserMessage(message)] if message.text == "persist me"));
            }
        }).await.unwrap();
    }
}
