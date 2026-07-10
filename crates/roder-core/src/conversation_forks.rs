//! Conversation forks (roadmap phases 90 + 81).
//!
//! Forks an existing thread into a child thread backed by a workspace fork
//! from any registered `ForkProvider` (default: `git-worktree`): the child
//! starts from the parent transcript (no side-effectful tool replay — only
//! conversation history records are copied) and all subsequent tool
//! execution resolves against the fork workspace because the child's
//! `ThreadMetadata.workspace` points at it. Cleanup is explicit and
//! path-confirmed; the parent workspace is never modified.

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::events::{
    EventEnvelope, RoderEvent, ThreadCreated, ThreadForkFailed, ThreadForkRemoved,
    ThreadForkRequested, ThreadForked, ThreadId, TurnId,
};
use roder_api::forks::{
    ForkPolicy, ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy, WorkspaceFork,
};
use roder_api::thread::{ThreadMetadata, ThreadStore};
use time::OffsetDateTime;

use crate::Runtime;
use crate::forks::DEFAULT_FORK_PROVIDER;

#[derive(Debug, Clone)]
pub struct ForkThreadRequest {
    pub parent_thread_id: ThreadId,
    /// User-facing fork name; the provider sanitizes it into its naming
    /// scheme (directories, branches, snapshot names).
    pub name: String,
    /// Fork at a specific parent turn; `None` forks at the latest turn.
    pub from_turn_id: Option<TurnId>,
    /// Fork provider id; `None` uses [`DEFAULT_FORK_PROVIDER`].
    pub provider_id: Option<String>,
    /// Provider-specific options (never secrets).
    pub provider_config: serde_json::Value,
}

impl ForkThreadRequest {
    pub fn new(parent_thread_id: ThreadId, name: impl Into<String>) -> Self {
        Self {
            parent_thread_id,
            name: name.into(),
            from_turn_id: None,
            provider_id: None,
            provider_config: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForkThreadOutcome {
    pub child: ThreadMetadata,
    pub warnings: Vec<String>,
}

impl Runtime {
    /// Seeds a long-lived collaboration agent with a safe subset of the parent
    /// conversation. Unlike `fork_thread`, this keeps the same workspace: only
    /// transcript/lifecycle records are copied, never executable tool or approval
    /// events.
    pub(crate) async fn seed_agent_thread_history(
        &self,
        parent_thread_id: &ThreadId,
        child_thread_id: &ThreadId,
        fork_turns: &str,
    ) -> anyhow::Result<()> {
        if fork_turns == "none" {
            return Ok(());
        }
        let Some(store) = self.thread_store.clone() else {
            return Ok(());
        };
        let Some(parent) = store.load_thread(parent_thread_id).await? else {
            return Ok(());
        };
        let mut events = seed_events_for_child(&parent.events, None)?;
        if fork_turns != "all" {
            let turn_count = fork_turns.parse::<usize>().map_err(|_| {
                anyhow::anyhow!("fork_turns must be one of none, all, or a positive integer")
            })?;
            anyhow::ensure!(turn_count > 0, "fork_turns integer must be positive");
            let mut ordered_turns = Vec::<TurnId>::new();
            for event in &events {
                if let Some(turn_id) = event.turn_id.as_ref()
                    && ordered_turns.last() != Some(turn_id)
                {
                    ordered_turns.push(turn_id.clone());
                }
            }
            let keep_from = ordered_turns.len().saturating_sub(turn_count);
            let kept = &ordered_turns[keep_from..];
            events.retain(|event| {
                event
                    .turn_id
                    .as_ref()
                    .is_some_and(|turn_id| kept.contains(turn_id))
            });
        }
        for event in &events {
            store.append_event(child_thread_id, event).await?;
        }
        Ok(())
    }

    /// Forks `parent_thread_id` into a new child thread backed by a fresh
    /// workspace fork of the parent workspace.
    pub async fn fork_thread(
        &self,
        request: ForkThreadRequest,
    ) -> anyhow::Result<ForkThreadOutcome> {
        self.emit(RoderEvent::ThreadForkRequested(ThreadForkRequested {
            parent_thread_id: request.parent_thread_id.clone(),
            name: request.name.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        match self.fork_thread_inner(&request).await {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.emit(RoderEvent::ThreadForkFailed(ThreadForkFailed {
                    parent_thread_id: request.parent_thread_id.clone(),
                    name: request.name.clone(),
                    message: error.to_string(),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                Err(error)
            }
        }
    }

    async fn fork_thread_inner(
        &self,
        request: &ForkThreadRequest,
    ) -> anyhow::Result<ForkThreadOutcome> {
        let store = self
            .thread_store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("conversation forks require a thread store"))?;
        let parent = store
            .load_thread(&request.parent_thread_id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("parent thread {} was not found", request.parent_thread_id)
            })?;
        let parent_metadata = parent.metadata.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "parent thread {} has no metadata to fork from",
                request.parent_thread_id
            )
        })?;

        // Materialize the workspace fork first; thread creation only
        // proceeds once an isolated workspace exists.
        let provider_id = request
            .provider_id
            .clone()
            .unwrap_or_else(|| DEFAULT_FORK_PROVIDER.to_string());
        let fork = self
            .create_workspace_fork(
                &provider_id,
                ForkRequest {
                    source_workspace: PathBuf::from(&parent_metadata.workspace),
                    name: Some(request.name.clone()),
                    reason: ForkReason::ConversationFork,
                    policy: ForkPolicy::default(),
                    provider_config: request.provider_config.clone(),
                },
            )
            .await?;

        let now = OffsetDateTime::now_utc();
        let seed_events = seed_events_for_child(&parent.events, request.from_turn_id.as_deref())?;
        let mut warnings = Vec::new();
        if request.from_turn_id.is_none() && seed_events.is_empty() && !parent.events.is_empty() {
            warnings.push(
                "parent thread has events but none were conversation records; the fork starts \
                 with an empty transcript"
                    .to_string(),
            );
        }

        let child_id = uuid::Uuid::new_v4().to_string();
        let child_metadata = ThreadMetadata {
            thread_id: child_id.clone(),
            title: Some(match &parent_metadata.title {
                Some(title) => format!("{title} (fork: {})", request.name),
                None => format!("fork: {}", request.name),
            }),
            workspace: fork.workspace.display().to_string(),
            // The fork workspace lives outside registered workspace roots.
            workspace_id: None,
            root_id: None,
            provider: parent_metadata.provider.clone(),
            model: parent_metadata.model.clone(),
            selection_mode: parent_metadata.selection_mode.clone(),
            tool_allowlist: parent_metadata.tool_allowlist.clone(),
            developer_instructions: parent_metadata.developer_instructions.clone(),
            external_tools: parent_metadata.external_tools.clone(),
            // Local workspace forks never inherit runner bindings.
            runner_destination: None,
            runner_state: None,
            runner_binding: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
            usage: None,
            parent_thread_id: Some(request.parent_thread_id.clone()),
            forked_from_turn_id: request.from_turn_id.clone(),
            workspace_fork: Some(fork.clone()),
        };

        if let Err(error) = self
            .seed_child_thread(&store, child_metadata.clone(), &child_id, seed_events)
            .await
        {
            // Best-effort cleanup so a failed fork does not leak a workspace.
            let _ = self
                .remove_workspace_fork(
                    &provider_id,
                    &fork.id,
                    RemoveForkPolicy {
                        confirm_workspace: fork.workspace.clone(),
                    },
                )
                .await;
            return Err(error);
        }

        self.emit(RoderEvent::ThreadCreated(ThreadCreated {
            thread_id: child_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.emit(RoderEvent::ThreadForked(ThreadForked {
            parent_thread_id: request.parent_thread_id.clone(),
            child_thread_id: child_id.clone(),
            fork,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        let child = store
            .load_thread_metadata(&child_id)
            .await?
            .unwrap_or(child_metadata);
        Ok(ForkThreadOutcome { child, warnings })
    }

    async fn seed_child_thread(
        &self,
        store: &Arc<dyn ThreadStore>,
        child_metadata: ThreadMetadata,
        child_id: &ThreadId,
        seed_events: Vec<EventEnvelope>,
    ) -> anyhow::Result<()> {
        store.create_thread(child_metadata).await?;
        for envelope in &seed_events {
            store.append_event(child_id, envelope).await?;
        }
        Ok(())
    }

    /**
     * Removes the workspace fork behind a forked thread. Destructive and
     * explicit: `confirm_path` must match the fork workspace exactly. The
     * thread itself is kept (status flips to `Removed`) so the conversation
     * stays readable.
     */
    pub async fn remove_thread_workspace_fork(
        &self,
        thread_id: &ThreadId,
        confirm_path: &str,
    ) -> anyhow::Result<WorkspaceFork> {
        let store = self
            .thread_store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("conversation forks require a thread store"))?;
        let mut metadata = store
            .load_thread_metadata(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} was not found"))?;
        let mut fork = metadata
            .workspace_fork
            .clone()
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} is not a workspace fork"))?;
        anyhow::ensure!(
            fork.status == ForkStatus::Active,
            "fork {} was already removed",
            fork.id
        );
        anyhow::ensure!(
            std::path::Path::new(confirm_path) == fork.workspace,
            "confirmation path does not match the fork workspace {}; removal is \
             path-confirmed to prevent accidental deletion",
            fork.workspace.display()
        );

        self.remove_workspace_fork(
            &fork.provider_id.clone(),
            &fork.id.clone(),
            RemoveForkPolicy {
                confirm_workspace: fork.workspace.clone(),
            },
        )
        .await?;

        fork.status = ForkStatus::Removed;
        metadata.workspace_fork = Some(fork.clone());
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;

        self.emit(RoderEvent::ThreadForkRemoved(ThreadForkRemoved {
            thread_id: thread_id.clone(),
            fork_id: fork.id.clone(),
            worktree_path: fork.workspace.display().to_string(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(fork)
    }
}

/**
 * Selects the parent events that seed the child transcript: only
 * conversation-history records (turn lifecycle and transcript items), never
 * tool/approval/audit events, so nothing side-effectful is replayed. When
 * `from_turn_id` is set, events after that turn's records are dropped.
 */
fn seed_events_for_child(
    events: &[EventEnvelope],
    from_turn_id: Option<&str>,
) -> anyhow::Result<Vec<EventEnvelope>> {
    let mut ordered: Vec<&EventEnvelope> = events.iter().collect();
    ordered.sort_by_key(|envelope| envelope.seq);

    let cutoff = match from_turn_id {
        Some(turn_id) => {
            let last = ordered
                .iter()
                .rposition(|envelope| envelope.turn_id.as_deref() == Some(turn_id))
                .ok_or_else(|| {
                    anyhow::anyhow!("turn {turn_id} was not found in the parent thread")
                })?;
            last + 1
        }
        None => ordered.len(),
    };

    Ok(ordered[..cutoff]
        .iter()
        .filter(|envelope| match &envelope.event {
            RoderEvent::TurnStarted(_)
            | RoderEvent::TurnCompleted(_)
            | RoderEvent::TurnFailed(_)
            | RoderEvent::TurnInterrupted(_) => true,
            RoderEvent::TranscriptItemAppended(event) => event
                .item
                .as_ref()
                .is_some_and(forkable_agent_transcript_item),
            _ => false,
        })
        .map(|envelope| (*envelope).clone())
        .collect())
}

fn forkable_agent_transcript_item(item: &roder_api::transcript::TranscriptItem) -> bool {
    match item {
        roder_api::transcript::TranscriptItem::UserMessage(_) => true,
        roder_api::transcript::TranscriptItem::AssistantMessage(message) => message
            .phase
            .as_deref()
            .is_none_or(|phase| phase.is_empty() || phase == "final_answer"),
        roder_api::transcript::TranscriptItem::ReasoningSummary(_)
        | roder_api::transcript::TranscriptItem::ToolCall(_)
        | roder_api::transcript::TranscriptItem::ToolResult(_)
        | roder_api::transcript::TranscriptItem::FileChange(_)
        | roder_api::transcript::TranscriptItem::ContextCompaction(_)
        | roder_api::transcript::TranscriptItem::Error(_)
        | roder_api::transcript::TranscriptItem::ProviderMetadata(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{EventSource, TranscriptItemAppended, TurnCompleted, TurnStarted};
    use roder_api::transcript::{
        ContextCompactionRecord, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };

    fn envelope(seq: u64, turn_id: &str, event: RoderEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: format!("event-{seq}"),
            seq,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: event.kind().to_string(),
            thread_id: Some("parent".to_string()),
            turn_id: Some(turn_id.to_string()),
            event,
        }
    }

    fn turn_events(seq: u64, turn_id: &str, text: &str) -> Vec<EventEnvelope> {
        vec![
            envelope(
                seq,
                turn_id,
                RoderEvent::TurnStarted(TurnStarted {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    runtime_profile: Default::default(),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
            envelope(
                seq + 1,
                turn_id,
                RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    item_type: "user_message".to_string(),
                    item_index: None,
                    item: Some(TranscriptItem::UserMessage(UserMessage::text(text))),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
            envelope(
                seq + 2,
                turn_id,
                RoderEvent::TurnCompleted(TurnCompleted {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    usage: None,
                    finish_reason: Some("stop".to_string()),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
        ]
    }

    #[test]
    fn seed_events_keep_conversation_records_only() {
        let mut events = turn_events(1, "turn-1", "hello");
        events.push(envelope(
            4,
            "turn-1",
            RoderEvent::ToolCallStarted(roder_api::events::ToolCallStarted {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                tool_id: "call-1".to_string(),
                tool_name: Some("shell".to_string()),
                display_payload: None,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            7,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "context_compaction".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ContextCompaction(ContextCompactionRecord {
                    summary: "private parent compaction".to_string(),
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            5,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "tool_call".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ToolCall(ToolCallRecord {
                    id: "spawn-call".to_string(),
                    name: "spawn_agent".to_string(),
                    arguments: "{}".to_string(),
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            6,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "tool_result".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ToolResult(ToolResultRecord {
                    id: "spawn-call".to_string(),
                    name: Some("spawn_agent".to_string()),
                    result: "spawned".to_string(),
                    display_payload: None,
                    is_error: false,
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));

        let seeded = seed_events_for_child(&events, None).unwrap();

        assert_eq!(seeded.len(), 3, "tool records must not be replayed");
        assert!(
            seeded
                .iter()
                .all(|envelope| !matches!(envelope.event, RoderEvent::ToolCallStarted(_)))
        );
        assert!(seeded.iter().all(|envelope| {
            !matches!(
                &envelope.event,
                RoderEvent::TranscriptItemAppended(event)
                    if matches!(
                        event.item,
                        Some(
                            TranscriptItem::ToolCall(_)
                                | TranscriptItem::ToolResult(_)
                                | TranscriptItem::ContextCompaction(_)
                        )
                    )
            )
        }));
    }

    #[test]
    fn seed_events_truncate_at_requested_turn() {
        let mut events = turn_events(1, "turn-1", "first");
        events.extend(turn_events(10, "turn-2", "second"));

        let seeded = seed_events_for_child(&events, Some("turn-1")).unwrap();
        assert_eq!(seeded.len(), 3);
        assert!(
            seeded
                .iter()
                .all(|envelope| envelope.turn_id.as_deref() == Some("turn-1"))
        );

        let error = seed_events_for_child(&events, Some("missing-turn")).unwrap_err();
        assert!(error.to_string().contains("missing-turn"));
    }
}
