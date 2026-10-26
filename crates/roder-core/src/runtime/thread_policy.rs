use super::*;
use roder_api::extension_state::{ExtensionStateRecord, ExtensionStoreScope};

const POLICY_EXTENSION: &str = "roder-core";
const POLICY_KEY: &str = "thread-policy-mode";

impl Runtime {
    /// Persist a thread-specific policy without changing other sessions' permissions.
    pub async fn set_thread_policy_mode(
        &self,
        thread_id: &str,
        mode: PolicyMode,
    ) -> anyhow::Result<()> {
        let previous_mode = self.effective_policy_mode_for_thread(thread_id).await;
        let mut modes = self.thread_policy_modes.write().await;
        if let Some(store) = &self.thread_store {
            let id = thread_id.to_string();
            anyhow::ensure!(
                store.load_thread_metadata(&id).await?.is_some(),
                "thread not found: {thread_id}"
            );
            store
                .append_extension_state(
                    &id,
                    &ExtensionStateRecord {
                        extension_id: POLICY_EXTENSION.into(),
                        key: POLICY_KEY.into(),
                        scope: ExtensionStoreScope::Thread {
                            thread_id: id.clone(),
                        },
                        schema_version: 1,
                        value: serde_json::to_value(mode)?,
                    },
                )
                .await?;
        }
        modes.insert(thread_id.into(), mode);
        drop(modes);
        self.emit(RoderEvent::PolicyModeChanged(PolicyModeChanged {
            thread_id: thread_id.into(),
            turn_id: None,
            previous_mode,
            new_mode: mode,
            reason: Some("thread mode selection".into()),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.auto_resolve_pending_tool_approvals().await;
        Ok(())
    }

    pub async fn thread_policy_mode(&self, thread_id: &str) -> anyhow::Result<Option<PolicyMode>> {
        let mut modes = self.thread_policy_modes.write().await;
        if let Some(mode) = modes.get(thread_id).copied() {
            return Ok(Some(mode));
        }
        let Some(store) = &self.thread_store else {
            return Ok(None);
        };
        let states = store.load_extension_states(&thread_id.to_string()).await?;
        let record = states
            .iter()
            .rev()
            .find(|state| state.extension_id == POLICY_EXTENSION && state.key == POLICY_KEY);
        let Some(record) = record else {
            return Ok(None);
        };
        anyhow::ensure!(
            record.schema_version == 1
                && record.scope
                    == (ExtensionStoreScope::Thread {
                        thread_id: thread_id.into()
                    }),
            "invalid thread policy state"
        );
        let mode = serde_json::from_value(record.value.clone())?;
        modes.insert(thread_id.into(), mode);
        Ok(Some(mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn thread_policy_plan_exit_changes_only_the_requesting_thread() {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
        runtime
            .set_thread_policy_mode("a", PolicyMode::Plan)
            .await
            .unwrap();
        runtime
            .set_thread_policy_mode("b", PolicyMode::Plan)
            .await
            .unwrap();
        runtime
            .record_pending_plan_exit(PendingPlanExit::new(
                "a".into(),
                "turn".into(),
                "exit".into(),
                PolicyMode::AcceptAll,
                Some("Approved implementation".into()),
                vec![],
            ))
            .await;
        runtime
            .resolve_pending_plan_exit("exit", true)
            .await
            .unwrap();
        assert_eq!(
            runtime.effective_policy_mode_for_thread("a").await,
            PolicyMode::AcceptAll
        );
        assert_eq!(
            runtime.effective_policy_mode_for_thread("b").await,
            PolicyMode::Plan
        );
        assert_eq!(runtime.status().await.policy_mode, PolicyMode::Default);
    }

    #[tokio::test]
    async fn thread_policy_denied_plan_exit_reports_scoped_mode() {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), Default::default()).unwrap();
        runtime
            .set_thread_policy_mode("a", PolicyMode::Plan)
            .await
            .unwrap();
        runtime
            .set_policy_mode(PolicyMode::Bypass, None)
            .await
            .unwrap();
        let mut events = runtime.subscribe_events();
        runtime
            .record_pending_plan_exit(PendingPlanExit::new(
                "a".into(),
                "turn".into(),
                "exit".into(),
                PolicyMode::Default,
                None,
                vec![],
            ))
            .await;
        runtime
            .resolve_pending_plan_exit("exit", false)
            .await
            .unwrap();
        loop {
            if let RoderEvent::PolicyExitPlanResolved(event) = events.recv().await.unwrap().event {
                assert!(!event.approved);
                assert_eq!(event.resolved_mode, PolicyMode::Plan);
                break;
            }
        }
        assert_eq!(
            runtime.effective_policy_mode_for_thread("a").await,
            PolicyMode::Plan
        );
        assert_eq!(runtime.status().await.policy_mode, PolicyMode::Bypass);
    }
}
