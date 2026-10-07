use super::*;

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[tokio::test]
    async fn cached_goal_compaction_uses_current_permissions() {
        let runtime = Runtime::fake().unwrap();
        let id = "goal-compaction-permissions".to_string();
        let template = runtime
            .compaction_request_template(
                &id,
                &"turn".into(),
                roder_api::catalog::PROVIDER_MOCK,
                "mock-model",
            )
            .await
            .unwrap();
        runtime
            .compaction_templates
            .write()
            .await
            .insert(id.clone(), template);
        runtime
            .thread_goal_set(
                &id,
                roder_api::goals::ThreadGoalPatch {
                    objective: Some("Finish the authorized task".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for (mode, expected) in [
            (PolicyMode::Bypass, "Full Access"),
            (PolicyMode::Default, "Default mode"),
        ] {
            runtime.set_policy_mode(mode, None).await.unwrap();
            let template = runtime
                .compaction_request_template(
                    &id,
                    &"turn".into(),
                    roder_api::catalog::PROVIDER_MOCK,
                    "mock-model",
                )
                .await
                .unwrap();
            assert!(
                template
                    .instructions
                    .developer
                    .as_ref()
                    .unwrap()
                    .contains(&format!("Goal permissions: {expected}"))
            );
            if mode == PolicyMode::Default {
                assert!(
                    !template
                        .instructions
                        .developer
                        .as_ref()
                        .unwrap()
                        .contains("Goal permissions: Full Access")
                );
            }
        }
    }
}

impl Runtime {
    pub(crate) async fn compaction_request_template(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
    ) -> anyhow::Result<AgentInferenceRequest> {
        let mode = self.effective_policy_mode_for_thread(thread_id).await;
        if let Some(mut template) = self
            .compaction_templates
            .read()
            .await
            .get(thread_id)
            .cloned()
        {
            template.instructions = self
                .goals
                .apply_goal_instructions(thread_id, template.instructions, mode)
                .await?;
            return Ok(template);
        }
        let cfg = self.status().await;
        let overrides = self.thread_turn_overrides(thread_id).await?;
        let profile = model_profile_for_provider_model(&cfg, provider, model);
        let tools = self.filtered_tool_specs(
            &cfg,
            model,
            provider,
            profile.as_ref(),
            &overrides.tool_allowlist,
            &overrides.external_tools,
        );
        let context = self.active_turn_contexts.read().await.get(turn_id).cloned();
        let mut instructions = context
            .as_ref()
            .map(|context| context.instructions.clone())
            .unwrap_or_else(crate::default_instructions);
        if let Some(developer) = overrides.developer_instructions {
            instructions
                .developer
                .get_or_insert_default()
                .push_str(&format!("\n{developer}"));
        }
        instructions.developer_context = context.and_then(|context| context.developer_context);
        instructions = self
            .goals
            .apply_goal_instructions(thread_id, instructions, mode)
            .await?;
        Ok(AgentInferenceRequest {
            model: ModelSelection {
                provider: provider.into(),
                model: model.into(),
            },
            instructions,
            transcript: Vec::new(),
            tools,
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: serde_json::json!({}),
        })
    }

    pub(crate) async fn compaction_steering(
        &self,
        turn_id: &TurnId,
    ) -> Option<tokio::sync::watch::Receiver<u64>> {
        let active = self.active_turns.read().await.get(turn_id).cloned()?;
        let mut receiver = active.steer_changed.subscribe();
        if !active.steers.lock().await.is_empty() {
            receiver.mark_changed();
        }
        Some(receiver)
    }
}
