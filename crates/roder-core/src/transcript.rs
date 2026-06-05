use roder_api::artifacts::{ContextArtifactKind, format_artifact_reference};
use roder_api::catalog::lookup_model;
use roder_api::context::{ContextBlockKind, ContextPlan, ContextQuery};
use roder_api::events::*;
use roder_api::retrieval::{RetrievalRoutePlan, RetrievalRoutePlanned};
use roder_api::transcript::{
    ContextCompactionRecord, ToolResultRecord, TranscriptItem, UserMessage,
};
use time::OffsetDateTime;

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
        let transcript = self
            .compact_transcript_if_needed(&req.thread_id, turn_id, model, transcript)
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
        Ok(out)
    }

    pub(crate) async fn compact_transcript_if_needed(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        model: &str,
        transcript: Vec<TranscriptItem>,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let cfg = self.status().await;
        let model_entry = lookup_model(model);
        let estimated_tokens = estimate_tokens(&transcript);
        let emergency_limit = model_entry
            .and_then(|entry| (entry.context_window > 0).then_some(entry.context_window));
        let threshold = cfg
            .auto_compact_token_limit
            .or_else(|| model_entry.map(|entry| entry.auto_compact_token_limit))
            .unwrap_or(0);
        let should_compact = if transcript_contains_context_limit_failure(&transcript) {
            true
        } else if model_entry.is_some_and(|entry| entry.supports_compaction) {
            emergency_limit.is_some_and(|limit| estimated_tokens >= limit)
        } else {
            threshold > 0 && estimated_tokens >= threshold
        };
        if !should_compact {
            return Ok(transcript);
        }
        let original_item_count = transcript.len() as u64;
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
        let suffix = transcript
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<TranscriptItem>>();
        let summary = if cfg.file_backed_dynamic_context {
            let history_json = serde_json::to_vec_pretty(&transcript)?;
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
            format!("{}\n\n{}", summarize_transcript(&transcript), reference)
        } else {
            summarize_transcript(&transcript)
        };
        let compaction = TranscriptItem::ContextCompaction(ContextCompactionRecord { summary });
        self.persist_turn_item(thread_id, turn_id, &compaction)
            .await?;
        let mut compacted = vec![compaction];
        compacted.extend(suffix);
        self.emit(RoderEvent::ContextCompactionRecorded(
            ContextCompactionRecorded {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                original_item_count,
                original_estimated_tokens,
                compacted_item_count: compacted.len() as u64,
                compacted_estimated_tokens: estimate_tokens(&compacted),
                file_backed: cfg.file_backed_dynamic_context,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(compacted)
    }
}

fn estimate_tokens(items: &[TranscriptItem]) -> u32 {
    let chars: usize = items.iter().map(item_text_len).sum();
    chars_to_tokens(chars)
}

fn estimate_text_tokens(text: &str) -> u32 {
    chars_to_tokens(text.len())
}

fn chars_to_tokens(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
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
        TranscriptItem::ProviderMetadata(value) => value.to_string().len(),
    }
}

fn transcript_contains_context_limit_failure(items: &[TranscriptItem]) -> bool {
    items.iter().any(|item| {
        let TranscriptItem::Error(error) = item else {
            return false;
        };
        let message = error.message.to_ascii_lowercase();
        message.contains("context window")
            || message.contains("input exceeds")
            || message.contains("response.incomplete")
    })
}

fn summarize_transcript(items: &[TranscriptItem]) -> String {
    let mut lines = vec!["Previous transcript was compacted. Key retained facts:".to_string()];
    for item in items.iter().take(items.len().saturating_sub(1)) {
        match item {
            TranscriptItem::UserMessage(message) => {
                lines.push(format!("- user: {}", truncate(&message.text)));
            }
            TranscriptItem::AssistantMessage(message) => {
                lines.push(format!("- assistant: {}", truncate(&message.text)));
            }
            TranscriptItem::ToolResult(ToolResultRecord { name, result, .. }) => {
                let name = name.as_deref().unwrap_or("tool");
                lines.push(format!("- {name} result: {}", truncate(result)));
            }
            TranscriptItem::ContextCompaction(compaction) => {
                lines.push(format!(
                    "- prior summary: {}",
                    truncate(&compaction.summary)
                ));
            }
            _ => {}
        }
    }
    lines.join("\n")
}

fn truncate(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 240;
    if normalized.len() <= LIMIT {
        normalized
    } else {
        format!("{}...", &normalized[..LIMIT])
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use roder_api::catalog::PROVIDER_MOCK;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::skills::{SkillExposure, SkillSelector};
    use roder_api::transcript::{AssistantMessage, ErrorRecord, UserMessage};
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use roder_skills::{SkillConfigRule, SkillRegistry, SkillRegistryOptions, SkillRoot};

    use crate::fake_provider::FakeInferenceEngine;
    use crate::runtime::{Runtime, RuntimeConfig, StartTurnRequest};

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
        assert!(!index.contains("vcs-snapshot"));
        assert!(texts.iter().any(|text| {
            text.starts_with("<skill name=\"vcs-snapshot\"") && text.contains("VCS status")
        }));
        let emitted = drain_events(&mut events);
        assert!(emitted.iter().any(|event| {
            matches!(
                event,
                RoderEvent::SkillIndexRendered(rendered)
                    if rendered.rendered_count == 1 && rendered.hidden_count >= 1
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
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
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
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
                    && summary.summary.contains("read_artifact")
        ));
        assert!(matches!(
            &compacted[1],
            TranscriptItem::UserMessage(message) if message.text == "current prompt"
        ));
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
                        && recorded.compacted_item_count == 2
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
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
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
                "gpt-5.5",
                transcript.clone(),
            )
            .await
            .unwrap();

        assert_eq!(compacted, transcript);
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
            .compact_transcript_if_needed(&thread_id, &"turn".to_string(), "gpt-5.5", transcript)
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            TranscriptItem::ContextCompaction(summary)
                if summary.summary.contains("Previous transcript was compacted")
        ));
        assert!(matches!(
            &compacted[1],
            TranscriptItem::UserMessage(message) if message.text == "continue"
        ));
        let _ = std::fs::remove_dir_all(thread_root);
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
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                roadmap_data_dir: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
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
