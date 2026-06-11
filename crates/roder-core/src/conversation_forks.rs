//! Native worktree conversation forks (roadmap phase 90).
//!
//! Forks an existing thread into a child thread backed by a local Git
//! worktree: the child starts from the parent transcript (no side-effectful
//! tool replay — only conversation history records are copied) and all
//! subsequent tool execution resolves against the child worktree because the
//! child's `ThreadMetadata.workspace` points at it. Cleanup is explicit and
//! path-confirmed; the parent workspace is never modified.

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::events::{
    EventEnvelope, RoderEvent, ThreadCreated, ThreadForkFailed, ThreadForkRemoved,
    ThreadForkRequested, ThreadForked, ThreadId, TurnId,
};
use roder_api::thread::{
    ThreadMetadata, ThreadStore, ThreadWorktreeFork, WorktreeForkBackend, WorktreeForkCleanup,
    WorktreeForkStatus,
};
use roder_ext_git::{GitWorktreeForkRequest, create_worktree_fork, remove_worktree_fork};
use time::OffsetDateTime;

use crate::Runtime;

#[derive(Debug, Clone)]
pub struct ForkThreadRequest {
    pub parent_thread_id: ThreadId,
    /// User-facing fork name; becomes the worktree directory and branch name.
    pub name: String,
    /// Fork at a specific parent turn; `None` forks at the latest turn.
    pub from_turn_id: Option<TurnId>,
}

#[derive(Debug, Clone)]
pub struct ForkThreadOutcome {
    pub child: ThreadMetadata,
    pub warnings: Vec<String>,
}

impl Runtime {
    /// Forks `parent_thread_id` into a new child thread backed by a fresh Git
    /// worktree of the parent workspace.
    pub async fn fork_thread_worktree(
        &self,
        request: ForkThreadRequest,
    ) -> anyhow::Result<ForkThreadOutcome> {
        self.emit(RoderEvent::ThreadForkRequested(ThreadForkRequested {
            parent_thread_id: request.parent_thread_id.clone(),
            name: request.name.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        match self.fork_thread_worktree_inner(&request).await {
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

    async fn fork_thread_worktree_inner(
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

        // Materialize the worktree first; thread creation only proceeds once
        // an isolated workspace exists.
        let worktree_request = GitWorktreeForkRequest {
            source_workspace: PathBuf::from(&parent_metadata.workspace),
            fork_name: request.name.clone(),
            base_dir: None,
        };
        let fork = tokio::task::spawn_blocking(move || create_worktree_fork(&worktree_request))
            .await
            .map_err(|err| anyhow::anyhow!("worktree fork task panicked: {err}"))??;

        let now = OffsetDateTime::now_utc();
        let worktree_fork = ThreadWorktreeFork {
            fork_id: fork.fork_id.clone(),
            backend: WorktreeForkBackend::GitWorktree,
            source_workspace: fork.source_workspace.display().to_string(),
            worktree_path: fork.worktree_path.display().to_string(),
            branch: fork.branch.clone(),
            source_branch: fork.source_branch.clone(),
            source_commit: fork.source_commit.clone(),
            created_at: now,
            status: WorktreeForkStatus::Active,
            cleanup: WorktreeForkCleanup::Explicit,
        };

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
            workspace: worktree_fork.worktree_path.clone(),
            // The worktree lives outside registered workspace roots.
            workspace_id: None,
            root_id: None,
            provider: parent_metadata.provider.clone(),
            model: parent_metadata.model.clone(),
            selection_mode: parent_metadata.selection_mode.clone(),
            tool_allowlist: parent_metadata.tool_allowlist.clone(),
            developer_instructions: parent_metadata.developer_instructions.clone(),
            external_tools: parent_metadata.external_tools.clone(),
            // Native worktree forks are local-only; runner bindings stay behind.
            runner_destination: None,
            runner_state: None,
            runner_binding: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
            usage: None,
            parent_thread_id: Some(request.parent_thread_id.clone()),
            forked_from_turn_id: request.from_turn_id.clone(),
            worktree_fork: Some(worktree_fork.clone()),
        };

        if let Err(error) = self
            .seed_child_thread(&store, child_metadata.clone(), &child_id, seed_events)
            .await
        {
            // Best-effort cleanup so a failed fork does not leak a worktree.
            let source = fork.source_workspace.clone();
            let path = fork.worktree_path.clone();
            let _ = tokio::task::spawn_blocking(move || remove_worktree_fork(&source, &path)).await;
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
            fork: worktree_fork,
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
     * Removes the worktree behind a forked thread. Destructive and explicit:
     * `confirm_path` must match the stored worktree path exactly, and only
     * Git-registered Roder worktrees are ever removed. The thread itself is
     * kept (status flips to `Removed`) so the conversation stays readable.
     */
    pub async fn remove_thread_worktree_fork(
        &self,
        thread_id: &ThreadId,
        confirm_path: &str,
    ) -> anyhow::Result<ThreadWorktreeFork> {
        let store = self
            .thread_store
            .clone()
            .ok_or_else(|| anyhow::anyhow!("conversation forks require a thread store"))?;
        let mut metadata = store
            .load_thread_metadata(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} was not found"))?;
        let mut fork = metadata
            .worktree_fork
            .clone()
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} is not a worktree fork"))?;
        anyhow::ensure!(
            fork.status == WorktreeForkStatus::Active,
            "fork {} was already removed",
            fork.fork_id
        );
        anyhow::ensure!(
            confirm_path == fork.worktree_path,
            "confirmation path does not match the fork worktree path {}; removal is \
             path-confirmed to prevent accidental deletion",
            fork.worktree_path
        );

        let source = PathBuf::from(&fork.source_workspace);
        let path = PathBuf::from(&fork.worktree_path);
        tokio::task::spawn_blocking(move || remove_worktree_fork(&source, &path))
            .await
            .map_err(|err| anyhow::anyhow!("worktree removal task panicked: {err}"))??;

        fork.status = WorktreeForkStatus::Removed;
        metadata.worktree_fork = Some(fork.clone());
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;

        self.emit(RoderEvent::ThreadForkRemoved(ThreadForkRemoved {
            thread_id: thread_id.clone(),
            fork_id: fork.fork_id.clone(),
            worktree_path: fork.worktree_path.clone(),
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
        .filter(|envelope| {
            matches!(
                envelope.event,
                RoderEvent::TurnStarted(_)
                    | RoderEvent::TranscriptItemAppended(_)
                    | RoderEvent::TurnCompleted(_)
                    | RoderEvent::TurnFailed(_)
                    | RoderEvent::TurnInterrupted(_)
            )
        })
        .map(|envelope| (*envelope).clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{EventSource, TranscriptItemAppended, TurnCompleted, TurnStarted};
    use roder_api::transcript::{TranscriptItem, UserMessage};

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

        let seeded = seed_events_for_child(&events, None).unwrap();

        assert_eq!(seeded.len(), 3, "tool events must not be replayed");
        assert!(
            seeded
                .iter()
                .all(|envelope| !matches!(envelope.event, RoderEvent::ToolCallStarted(_)))
        );
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
