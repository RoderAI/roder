use roder_api::artifacts::{ContextArtifactKind, format_artifact_reference};
use roder_api::context::{ContextBlockKind, ContextPlan, ContextQuery};
use roder_api::events::*;
use roder_api::retrieval::{RetrievalRoutePlan, RetrievalRoutePlanned};
use roder_api::transcript::{TranscriptItem, UserMessage};
use time::OffsetDateTime;

use crate::compaction::{
    CompactionOptions, CompactionSkipReason, build_compaction_record, compaction_skip_reason,
    estimate_prompt_tokens, format_llm_compaction_summary, model_entry_for_compaction,
    prune_avoids_full_compaction, prune_tool_outputs_in_transcript, select_compaction_suffix,
    head_items_for_summary_prompt, split_transcript_for_summarization, summarize_transcript,
    trim_to_last_compaction_boundary,
};
use crate::runtime::{Runtime, StartTurnRequest};
use roder_api::artifacts::CreateArtifactRequest;

impl Runtime {
    pub(crate) async fn transcript_for_turn(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        model: &str,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let context_plan = self.start_context_assembly(req, turn_id).await?;
        let mut transcript = Vec::new();
        for block in context_plan.blocks {
            if matches!(
                block.kind,
                ContextBlockKind::Instruction
                    | ContextBlockKind::Memory
                    | ContextBlockKind::Knowledge
                    | ContextBlockKind::RepositoryFact
                    | ContextBlockKind::RetrievedDocument
                    | ContextBlockKind::PriorSummary
                    | ContextBlockKind::EntrypointHint
            ) {
                transcript.push(TranscriptItem::UserMessage(UserMessage {
                    text: block.text,
                    images: Vec::new(),
                }));
            }
        }
        transcript.extend(self.prior_transcript(&req.thread_id, turn_id).await?);
        transcript.push(TranscriptItem::UserMessage(UserMessage {
            text: req.message.clone(),
            images: req.images.clone(),
        }));
        let cfg = self.status().await;
        let provider = req
            .provider_override
            .as_deref()
            .unwrap_or(cfg.default_provider.as_str());
        let transcript = self
            .compact_transcript_if_needed(
                &req.thread_id,
                turn_id,
                provider,
                model,
                transcript,
                CompactionOptions::default(),
            )
            .await?;
        self.complete_context_assembly(req, turn_id, &transcript)
            .await;
        Ok(transcript)
    }

    async fn start_context_assembly(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
    ) -> anyhow::Result<ContextPlan> {
        self.emit(RoderEvent::ContextAssemblyStarted(ContextAssemblyStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        let query = ContextQuery {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            prompt: req.message.clone(),
            workspace: Some(req.workspace.clone()),
            token_budget: None,
        };
        let mut blocks = self.skill_context_blocks(req, turn_id).await;
        for provider in &self.registry.context_providers {
            for block in provider.blocks(&query).await? {
                self.emit(RoderEvent::ContextBlockAdded(ContextBlockAdded {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    block_type: format!("{:?}", block.kind),
                    byte_count: block.text.len() as u64,
                    estimated_tokens: estimate_text_tokens(&block.text),
                    priority: block.priority,
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                blocks.push(block);
            }
        }
        let plan = if self.registry.context_planners.is_empty() {
            ContextPlan { blocks }
        } else {
            let mut plan = ContextPlan { blocks };
            for planner in &self.registry.context_planners {
                plan = planner.plan(&query, plan.blocks).await?;
            }
            plan
        };
        for block in plan
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, ContextBlockKind::EntrypointHint))
        {
            self.emit(RoderEvent::ContextEntrypointCandidatesInjected(
                ContextEntrypointCandidatesInjected {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    candidate_count: block
                        .metadata
                        .get("candidate_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    block_byte_count: block.text.len() as u64,
                    estimated_tokens: estimate_text_tokens(&block.text),
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }
        for block in plan
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, ContextBlockKind::RetrievalHint))
        {
            if let Some(value) = block.metadata.get("retrievalPlan")
                && let Ok(plan) = serde_json::from_value::<RetrievalRoutePlan>(value.clone())
            {
                self.emit(RoderEvent::RetrievalRoutePlanned(RetrievalRoutePlanned {
                    plan,
                }))
                .await;
            }
        }
        Ok(plan)
    }

    async fn complete_context_assembly(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        transcript: &[TranscriptItem],
    ) {
        let block_count = transcript.len() as u64;
        let total_byte_count = transcript
            .iter()
            .map(item_text_len)
            .map(|len| len as u64)
            .sum::<u64>();
        let prompt_estimated_tokens = estimate_tokens(transcript);
        self.emit(RoderEvent::ContextAssemblyCompleted(
            ContextAssemblyCompleted {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                block_count,
                total_byte_count,
                estimated_tokens: prompt_estimated_tokens,
                prompt_estimated_tokens,
                token_budget: None,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    async fn prior_transcript(
        &self,
        thread_id: &ThreadId,
        current_turn_id: &TurnId,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let Some(store) = &self.thread_store else {
            return Ok(Vec::new());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for turn in snapshot.turns {
            if &turn.turn_id == current_turn_id {
                continue;
            }
            out.extend(turn.items);
        }
        Ok(trim_to_last_compaction_boundary(out))
    }

    pub(crate) async fn compact_transcript_if_needed(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        transcript: Vec<TranscriptItem>,
        options: CompactionOptions,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let cfg = self.status().await;
        let model_entry = model_entry_for_compaction(provider, model);
        let threshold = cfg.auto_compact_token_limit;
        if let Some(reason) =
            compaction_skip_reason(&transcript, model_entry, threshold, &options)
        {
            self.emit_compaction_skipped(
                thread_id,
                turn_id,
                reason,
                estimate_prompt_tokens(&transcript),
                threshold,
                None,
            )
            .await;
            return Ok(transcript);
        }

        let mut working = transcript;
        let prune_result = prune_tool_outputs_in_transcript(&working);
        let pruned_tool_count = prune_result.pruned_tool_count;
        if pruned_tool_count > 0 {
            let tokens_saved = prune_result.tokens_saved;
            working = prune_result.items;
            if !options.force
                && tokens_saved >= crate::compaction::TOOL_OUTPUT_PRUNE_MIN_SAVINGS_TOKENS
                && prune_avoids_full_compaction(
                    &crate::compaction::ToolOutputPruneResult {
                        items: working.clone(),
                        pruned_tool_count,
                        tokens_saved,
                    },
                    model_entry,
                    threshold,
                )
            {
                self.emit_compaction_skipped(
                    thread_id,
                    turn_id,
                    CompactionSkipReason::PruneSufficient,
                    estimate_prompt_tokens(&working),
                    threshold,
                    Some(pruned_tool_count),
                )
                .await;
                return Ok(working);
            }
        }

        if let Some(reason) =
            compaction_skip_reason(&working, model_entry, threshold, &options)
        {
            self.emit_compaction_skipped(
                thread_id,
                turn_id,
                reason,
                estimate_prompt_tokens(&working),
                threshold,
                Some(pruned_tool_count),
            )
            .await;
            return Ok(working);
        }

        let estimated_tokens = estimate_prompt_tokens(&working);
        let original_item_count = working.len() as u64;
        let original_estimated_tokens = estimated_tokens;
        self.emit(RoderEvent::ContextCompactionStarted(
            ContextCompactionStarted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                original_item_count,
                original_estimated_tokens,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;

        let split = split_transcript_for_summarization(&working);
        let suffix = if split.tail.is_empty() {
            select_compaction_suffix(&working)
        } else {
            split.tail
        };
        let summary_head = head_items_for_summary_prompt(&split.head);
        let (summary, strategy) = self
            .build_compaction_summary(
                thread_id,
                turn_id,
                provider,
                model,
                &summary_head,
                &working,
                options.preserve_hint.as_deref(),
                cfg.file_backed_dynamic_context,
            )
            .await?;
        let compaction = build_compaction_record(summary);
        self.persist_turn_item(thread_id, turn_id, &compaction)
            .await?;
        let mut compacted = vec![compaction];
        compacted.extend(suffix);
        self.record_compaction_hysteresis(thread_id, original_estimated_tokens);
        self.emit(RoderEvent::ContextCompactionRecorded(
            ContextCompactionRecorded {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                original_item_count,
                original_estimated_tokens,
                compacted_item_count: compacted.len() as u64,
                compacted_estimated_tokens: estimate_prompt_tokens(&compacted),
                file_backed: cfg.file_backed_dynamic_context,
                strategy: Some(strategy),
                pruned_tool_count: Some(pruned_tool_count),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(compacted)
    }

    async fn emit_compaction_skipped(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        reason: CompactionSkipReason,
        estimated_tokens: u32,
        threshold: Option<u32>,
        pruned_tool_count: Option<u32>,
    ) {
        if matches!(
            reason,
            CompactionSkipReason::BelowThreshold | CompactionSkipReason::AlreadyCompactedThisTurn
        ) {
            return;
        }
        self.emit(RoderEvent::ContextCompactionSkipped(
            ContextCompactionSkipped {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                reason: reason.as_str().to_string(),
                estimated_tokens,
                threshold,
                pruned_tool_count,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    async fn build_compaction_summary(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        head: &[TranscriptItem],
        full_transcript: &[TranscriptItem],
        preserve_hint: Option<&str>,
        file_backed: bool,
    ) -> anyhow::Result<(String, String)> {
        let mut used_llm = false;
        let mut summary = if head.is_empty() {
            summarize_transcript(full_transcript)
        } else if let Some(llm_summary) = self
            .summarize_compaction_head(provider, model, head, preserve_hint)
            .await?
        {
            used_llm = true;
            format_llm_compaction_summary(&llm_summary)
        } else {
            summarize_transcript(full_transcript)
        };
        let mut strategy = if used_llm {
            "llm".to_string()
        } else {
            "deterministic".to_string()
        };

        if file_backed {
            let history_json = serde_json::to_vec_pretty(full_transcript)?;
            let artifact = self.context_artifacts().create(CreateArtifactRequest {
                kind: ContextArtifactKind::ChatHistory,
                thread_id,
                turn_id,
                source_tool_id: None,
                label: Some("pre-compaction transcript"),
                bytes: &history_json,
            })?;
            self.emit(RoderEvent::ContextArtifactCreated(ContextArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            let reference = format_artifact_reference(&artifact, "pre-compaction transcript");
            summary = format!("{summary}\n\n{reference}");
            if strategy == "llm" {
                strategy = "llm_file_backed".to_string();
            } else {
                strategy = "deterministic_file_backed".to_string();
            }
        }
        Ok((summary, strategy))
    }
}

fn estimate_tokens(items: &[TranscriptItem]) -> u32 {
    estimate_prompt_tokens(items)
}

fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

fn item_text_len(item: &TranscriptItem) -> usize {
    match item {
        TranscriptItem::UserMessage(message) => message.text.len(),
        TranscriptItem::AssistantMessage(message) => message.text.len(),
        TranscriptItem::ReasoningSummary(summary) => summary.text.len(),
        TranscriptItem::ToolCall(call) => call.arguments.len() + call.name.len(),
        TranscriptItem::ToolResult(result) => result.result.len(),
        TranscriptItem::FileChange(change) => change.path.len() + change.change_type.len(),
        TranscriptItem::ContextCompaction(compaction) => compaction.summary.len(),
        TranscriptItem::Error(error) => error.message.len(),
        TranscriptItem::ProviderMetadata(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use roder_api::catalog::PROVIDER_MOCK;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::skills::{SkillExposure, SkillSelector};
    use roder_api::transcript::{
        AssistantMessage, ErrorRecord, ToolResultRecord, UserMessage,
    };
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use roder_skills::{SkillConfigRule, SkillRegistry, SkillRegistryOptions, SkillRoot};

    use crate::compaction::{
        build_compaction_record, should_compact_transcript, truncate,
        transcript_contains_context_limit_failure, CompactionOptions,
    };
    use crate::fake_provider::FakeInferenceEngine;
    use crate::runtime::{
        DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS, Runtime, RuntimeConfig, StartTurnRequest,
    };

    use super::*;

    fn test_workspace() -> String {
        std::env::current_dir().unwrap().display().to_string()
    }

    #[tokio::test]
    async fn skills_context_renders_global_index_and_direct_invocation() {
        let workspace = fixture_dir("skills-context");
        write_skill(
            &workspace.join(".agents/skills/review"),
            "review",
            "Review code changes",
            SkillExposure::Global,
        );
        let skills = SkillRegistry::load(SkillRegistryOptions {
            workspace: workspace.clone(),
            include_builtins: true,
            roots: vec![SkillRoot::workspace(
                workspace.join(".agents/skills"),
                "workspace://.agents/skills",
            )],
            workflow_imports: Vec::new(),
            config_rules: Vec::new(),
        });
        let runtime = runtime_with_skills(skills).await;
        let mut events = runtime.subscribe_events();

        let transcript = runtime
            .transcript_for_turn(
                &turn_request("thread-skills", "please use ${vcs-snapshot}"),
                &"turn-skills".to_string(),
                "mock",
            )
            .await
            .unwrap();
        let texts = transcript_texts(&transcript);
        let index = texts
            .iter()
            .find(|text| text.starts_with("<skills>"))
            .expect("global skill index");

        assert!(index.contains("review"));
        assert!(index.contains("roder-config"));
        assert!(!index.contains("vcs-snapshot"));
        assert!(texts.iter().any(|text| {
            text.starts_with("<skill name=\"vcs-snapshot\"") && text.contains("VCS status")
        }));
        let emitted = drain_events(&mut events);
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::SkillIndexRendered(rendered)
                    if rendered.rendered_count == 2 && rendered.hidden_count >= 1
            )
        }));
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::SkillInvoked(invoked)
                    if invoked.descriptor.name == "vcs-snapshot"
            )
        }));
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::ContextAssemblyCompleted(completed)
                    if completed.prompt_estimated_tokens > 0
                        && completed.prompt_estimated_tokens == completed.estimated_tokens
            )
        }));
    }

    #[tokio::test]
    async fn skills_disabled_direct_skill_does_not_inject() {
        let skills = SkillRegistry::load(SkillRegistryOptions {
            workspace: PathBuf::new(),
            include_builtins: true,
            roots: Vec::new(),
            workflow_imports: Vec::new(),
            config_rules: vec![SkillConfigRule {
                name: Some("vcs-snapshot".to_string()),
                path: None,
                enabled: Some(false),
                exposure: None,
            }],
        });
        let runtime = runtime_with_skills(skills).await;
        let mut events = runtime.subscribe_events();

        let transcript = runtime
            .transcript_for_turn(
                &turn_request("thread-disabled", "please use ${vcs-snapshot}"),
                &"turn-disabled".to_string(),
                "mock",
            )
            .await
            .unwrap();
        let texts = transcript_texts(&transcript);

        assert!(!texts.iter().any(|text| {
            text.starts_with("<skill name=\"vcs-snapshot\"") && text.contains("VCS status")
        }));
        let emitted = drain_events(&mut events);
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::SkillSkipped(skipped)
                    if skipped.selector == (SkillSelector::Name { name: "vcs-snapshot".to_string() })
                        && skipped.reason.contains("disabled")
            )
        }));
    }

    #[tokio::test]
    async fn context_compaction_writes_history_artifact_and_emits_metrics() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let thread_root =
            std::env::temp_dir().join(format!("roder-thread-artifacts-{}", uuid::Uuid::new_v4()));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                tool_search: Default::default(),
                provider_tool_search: std::collections::HashMap::new(),
                model_tool_search: std::collections::HashMap::new(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                external_tool_timeout_seconds: DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS,
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                inference_router: crate::inference_routing::RuntimeInferenceRouterConfig::default(),
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                media_generation: Default::default(),
            },
        )
        .unwrap();
        let thread_id = runtime
            .create_thread(Some("Context compaction".to_string()))
            .await
            .unwrap()
            .thread_id;
        let mut events = runtime.subscribe_events();
        let compacted = runtime
            .compact_transcript_if_needed(
                &thread_id,
                &"turn".to_string(),
                PROVIDER_MOCK,
                "mock",
                vec![
                    TranscriptItem::UserMessage(UserMessage {
                        text: "very large old context".repeat(20),
                        images: Vec::new(),
                    }),
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text: "old answer".to_string(),
                        phase: None,
                    }),
                    TranscriptItem::UserMessage(UserMessage {
                        text: "current prompt".to_string(),
                        images: Vec::new(),
                    }),
                ],
                CompactionOptions::default(),
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
                    && summary.summary.contains("read_artifact")
        ));
        assert!(compacted.iter().any(|item| matches!(
            item,
            TranscriptItem::UserMessage(message) if message.text == "current prompt"
        )));
        let artifacts = runtime
            .context_artifacts()
            .list_artifacts(&thread_id)
            .unwrap();
        let history = artifacts
            .iter()
            .find(|artifact| {
                artifact.kind == roder_api::artifacts::ContextArtifactKind::ChatHistory
            })
            .unwrap();
        let grep = runtime
            .context_artifacts()
            .grep_artifact(&thread_id, &history.id, "old answer", 0, 10)
            .unwrap();
        assert_eq!(grep.total_matches, 1);
        assert!(
            history.store_path.starts_with(
                thread_root
                    .join(&thread_id)
                    .join("artifacts")
                    .join("turn")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        let emitted = drain_events(&mut events);
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::ContextCompactionRecorded(recorded)
                    if recorded.original_item_count == 3
                        && recorded.compacted_item_count >= 2
                        && recorded.original_estimated_tokens > 0
                        && recorded.compacted_estimated_tokens > 0
                        && recorded.file_backed
            )
        }));
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn openai_server_side_compaction_models_skip_local_summary_compaction() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                tool_search: Default::default(),
                provider_tool_search: std::collections::HashMap::new(),
                model_tool_search: std::collections::HashMap::new(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                external_tool_timeout_seconds: DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS,
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                inference_router: crate::inference_routing::RuntimeInferenceRouterConfig::default(),
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                media_generation: Default::default(),
            },
        )
        .unwrap();
        let transcript = vec![
            TranscriptItem::UserMessage(UserMessage {
                text: "very large old context".repeat(20),
                images: Vec::new(),
            }),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "old answer".to_string(),
                phase: None,
            }),
            TranscriptItem::UserMessage(UserMessage {
                text: "current prompt".to_string(),
                images: Vec::new(),
            }),
        ];

        let compacted = runtime
            .compact_transcript_if_needed(
                &"thread".to_string(),
                &"turn".to_string(),
                PROVIDER_MOCK,
                "gpt-5.5",
                transcript.clone(),
                CompactionOptions::default(),
            )
            .await
            .unwrap();

        assert_eq!(compacted, transcript);
    }

    #[test]
    fn truncate_does_not_split_multibyte_chars() {
        // An em-dash is 3 bytes; place it so the 240-byte limit lands mid-char.
        let text = format!("{}\u{2014}tail", "a".repeat(238));
        let truncated = truncate(&text);
        assert!(truncated.ends_with("..."));
        // Must not panic and must remain valid UTF-8 below the limit.
        assert!(truncated.len() <= 240 + 3);
    }

    #[test]
    fn prompt_too_long_errors_are_recognized_as_context_limit_failures() {
        for message in [
            "Prompt is too long: 1048576 tokens > 1000000 maximum",
            "API Error: 400 prompt too long",
            "Your input exceeds the context window of this model.",
        ] {
            let items = vec![TranscriptItem::Error(ErrorRecord {
                message: message.to_string(),
            })];
            assert!(
                transcript_contains_context_limit_failure(&items),
                "expected context-limit failure for message: {message}"
            );
        }
    }

    #[tokio::test]
    async fn claude_code_models_compact_locally_on_the_fly() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-claude-code-compaction-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                // Force the proactive threshold low so the test does not have to
                // build a million-token transcript. Claude Code models must honor
                // this local threshold because the provider has no server-side
                // compaction of its own.
                auto_compact_token_limit: Some(1),
                file_backed_dynamic_context: true,
                ..RuntimeConfig::default()
            },
        )
        .unwrap();
        let thread_id = runtime
            .create_thread(Some("Claude Code compaction".to_string()))
            .await
            .unwrap()
            .thread_id;
        let transcript = vec![
            TranscriptItem::UserMessage(UserMessage {
                text: "very large old context".repeat(20),
                images: Vec::new(),
            }),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "old answer".to_string(),
                phase: None,
            }),
            TranscriptItem::UserMessage(UserMessage {
                text: "current prompt".to_string(),
                images: Vec::new(),
            }),
        ];

        let compacted = runtime
            .compact_transcript_if_needed(
                &thread_id,
                &"turn".to_string(),
                PROVIDER_MOCK,
                "sonnet",
                transcript,
                CompactionOptions::default(),
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
        ));
        assert!(compacted.iter().any(|item| matches!(
            item,
            TranscriptItem::UserMessage(message) if message.text == "current prompt"
        )));
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn context_window_failure_forces_local_compaction_on_server_side_models() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-context-failure-compaction-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                file_backed_dynamic_context: true,
                ..RuntimeConfig::default()
            },
        )
        .unwrap();
        let thread_id = runtime
            .create_thread(Some("Context failure compaction".to_string()))
            .await
            .unwrap()
            .thread_id;
        let transcript = vec![
            TranscriptItem::UserMessage(UserMessage {
                text: "work already done ".repeat(20),
                images: Vec::new(),
            }),
            TranscriptItem::Error(ErrorRecord {
                message: "Your input exceeds the context window of this model. Please adjust your input and try again."
                    .to_string(),
            }),
            TranscriptItem::UserMessage(UserMessage {
                text: "continue".to_string(),
                images: Vec::new(),
            }),
        ];

        let compacted = runtime
            .compact_transcript_if_needed(
                &thread_id,
                &"turn".to_string(),
                PROVIDER_MOCK,
                "gpt-5.5",
                transcript,
                CompactionOptions::default(),
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
        ));
        assert!(compacted.iter().any(|item| matches!(
            item,
            TranscriptItem::UserMessage(message) if message.text == "continue"
        )));
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn prune_can_avoid_full_compaction_for_tool_heavy_transcript() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                auto_compact_token_limit: Some(50_000),
                ..RuntimeConfig::default()
            },
        )
        .unwrap();
        let mut events = runtime.subscribe_events();
        let transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("start")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "old".to_string(),
                name: Some("grep".to_string()),
                result: "x".repeat(120_000),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::UserMessage(UserMessage::text("current")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "recent-buffer".to_string(),
                name: Some("read".to_string()),
                result: "y".repeat(180_000),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "new".to_string(),
                name: Some("read".to_string()),
                result: "fresh".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];
        let result = runtime
            .compact_transcript_if_needed(
                &"thread".to_string(),
                &"turn".to_string(),
                PROVIDER_MOCK,
                "mock",
                transcript,
                CompactionOptions::default(),
            )
            .await
            .unwrap();
        assert!(!result.iter().any(|item| {
            matches!(item, TranscriptItem::ContextCompaction(_))
        }));
        let emitted = drain_events(&mut events);
        assert!(emitted.iter().any(|event| matches!(
            event,
            RoderEvent::ContextCompactionSkipped(skipped)
                if skipped.reason == "prune_sufficient"
        )));
    }

    #[tokio::test]
    async fn force_compact_uses_llm_snapshot_when_available() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-force-compact-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                auto_compact_token_limit: Some(1),
                file_backed_dynamic_context: false,
                ..RuntimeConfig::default()
            },
        )
        .unwrap();
        let thread_id = runtime
            .create_thread(Some("Force compact".to_string()))
            .await
            .unwrap()
            .thread_id;
        runtime
            .persist_turn_item(
                &thread_id,
                &"turn-1".to_string(),
                &TranscriptItem::UserMessage(UserMessage::text("old".repeat(200))),
            )
            .await
            .unwrap();
        runtime
            .persist_turn_item(
                &thread_id,
                &"turn-1".to_string(),
                &TranscriptItem::UserMessage(UserMessage::text("current")),
            )
            .await
            .unwrap();
        let outcome = runtime
            .force_compact_thread(
                &thread_id,
                &"turn-1".to_string(),
                Some("keep current prompt".to_string()),
            )
            .await
            .unwrap();
        assert!(outcome.compacted);
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn repeat_compaction_within_turn_is_blocked_after_first_summary() {
        let items = vec![
            build_compaction_record("summary".to_string()),
            TranscriptItem::UserMessage(UserMessage::text("x".repeat(20_000))),
        ];
        let options = CompactionOptions {
            allow_repeat: false,
            ..CompactionOptions::default()
        };
        assert!(!should_compact_transcript(&items, None, Some(1), &options));
    }

    #[tokio::test]
    async fn compaction_without_file_backed_context_keeps_legacy_summary_only() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-thread-artifacts-disabled-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                file_backed_dynamic_context: false,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                tool_search: Default::default(),
                provider_tool_search: std::collections::HashMap::new(),
                model_tool_search: std::collections::HashMap::new(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                external_tool_timeout_seconds: DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS,
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                roadmap_data_dir: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                inference_router: crate::inference_routing::RuntimeInferenceRouterConfig::default(),
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                media_generation: Default::default(),
            },
        )
        .unwrap();
        let thread_id = runtime
            .create_thread(Some("Legacy compaction".to_string()))
            .await
            .unwrap()
            .thread_id;

        let compacted = runtime
            .compact_transcript_if_needed(
                &thread_id,
                &"turn".to_string(),
                PROVIDER_MOCK,
                "mock",
                vec![
                    TranscriptItem::UserMessage(UserMessage {
                        text: "very large old context".repeat(20),
                        images: Vec::new(),
                    }),
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text: "old answer".to_string(),
                        phase: None,
                    }),
                    TranscriptItem::UserMessage(UserMessage {
                        text: "current prompt".to_string(),
                        images: Vec::new(),
                    }),
                ],
                CompactionOptions::default(),
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
                    && !summary.summary.contains("read_artifact")
        ));
        assert!(
            runtime
                .context_artifacts()
                .list_artifacts(&thread_id)
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(thread_root);
    }

    fn drain_events(
        rx: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    ) -> Vec<RoderEvent> {
        let mut events = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(envelope) => events.push(envelope.event),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            }
        }
        events
    }

    async fn runtime_with_skills(skills: SkillRegistry) -> Runtime {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        runtime.set_skills(skills).await;
        runtime
    }

    fn turn_request(thread_id: &str, message: &str) -> StartTurnRequest {
        StartTurnRequest {
            thread_id: thread_id.to_string(),
            message: message.to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: test_workspace(),
            instructions: Default::default(),
            developer_context: None,
            task_ledger_required: false,
        }
    }

    fn transcript_texts(transcript: &[TranscriptItem]) -> Vec<&str> {
        transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::UserMessage(message) => Some(message.text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn write_skill(dir: &Path, name: &str, description: &str, exposure: SkillExposure) {
        std::fs::create_dir_all(dir).unwrap();
        let exposure = match exposure {
            SkillExposure::Global => "global",
            SkillExposure::DirectOnly => "direct_only",
        };
        std::fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\nexposure: {exposure}\n---\nBody for {name}\n"
            ),
        )
        .unwrap();
    }

    fn fixture_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("roder-core-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
