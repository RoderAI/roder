//! Runs a review as a detached, read-only sub-turn.
//!
//! Roder has no one-shot sub-agent with a per-call system prompt, so a review
//! is a child thread plus a single [`Runtime::start_turn`] whose
//! [`InstructionBundle::system`] is replaced by the rubric. The thread is
//! created with a read-only `tool_allowlist`, which is enforced both when tools
//! are advertised and when a call is dispatched — the global policy mode is
//! never touched, so a review can never widen what the parent thread may do.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use roder_api::events::{
    EventEnvelope, ReviewCompleted, ReviewFailed, ReviewStarted, RoderEvent, ThreadId, TurnId,
};
use roder_api::inference::{InferenceEvent, InstructionBundle};
use roder_api::review::{ReviewId, ReviewOutput, ReviewRequest, ReviewTarget};
use roder_api::transcript::{AssistantMessage, TranscriptItem, UserMessage};
use roder_api::version_control::{
    VcsProvider, VcsProviderResolution, VcsProviderResolver, VcsResolveRequest,
};
use time::OffsetDateTime;
use tokio::sync::broadcast;

use crate::review::REVIEW_RUBRIC;
use crate::review::parse::parse_review_output;
use crate::review::prompt::resolve_review_request;
use crate::review::render::{
    render_review_output_text, synthetic_user_action, synthetic_user_action_interrupted,
};
use crate::runtime::{CreateThreadRequest, FINAL_ANSWER_PHASE, Runtime, StartTurnRequest};

/// The only tools a review thread may call.
///
/// `shell` is included because `git diff`/`git log` is how a reviewer sees a
/// change; every tool that writes files, edits code, applies patches, or spawns
/// agents is deliberately absent. Names are matched exactly against the tool
/// registry (`allowlist_permits`), so they must stay in sync with
/// `roder-tools`.
pub const REVIEW_TOOL_ALLOWLIST: &[&str] = &["read_file", "list_files", "grep", "glob", "shell"];

/// How many completed reviews stay resolvable by id for a later publish.
const REVIEW_HISTORY_LIMIT: usize = 64;

const REVIEW_INTERRUPTED_MESSAGE: &str =
    "Review was interrupted. Re-run `/review` and wait for it to complete.";

#[derive(Debug, Clone)]
pub struct StartReviewRequest {
    /// Thread the review was requested from; the review runs in a child of it.
    pub thread_id: ThreadId,
    pub target: ReviewTarget,
    pub workspace: String,
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewHandle {
    pub review_id: ReviewId,
    pub review_thread_id: ThreadId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone)]
pub struct ReviewCompletion {
    pub handle: ReviewHandle,
    pub output: ReviewOutput,
}

/// Everything a publisher needs about a finished review, kept in memory so
/// `review/publish` can act on a review id alone.
#[derive(Debug, Clone)]
pub struct ReviewRecord {
    pub review_id: ReviewId,
    pub parent_thread_id: ThreadId,
    pub review_thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request: ReviewRequest,
    pub output: ReviewOutput,
    pub workspace_root: PathBuf,
}

/// A review whose turn is running but whose findings have not been collected.
struct StartedReview {
    handle: ReviewHandle,
    request: ReviewRequest,
    parent_thread_id: ThreadId,
    events: broadcast::Receiver<EventEnvelope>,
}

impl Runtime {
    /// Starts a review and returns as soon as its turn is admitted. Findings
    /// arrive as [`RoderEvent::ReviewCompleted`].
    pub async fn start_review(
        self: &Arc<Self>,
        req: StartReviewRequest,
    ) -> anyhow::Result<ReviewHandle> {
        let started = self.begin_review(req).await?;
        let handle = started.handle.clone();
        let runtime = Arc::clone(self);
        // `finish_review` reports its own failure as `ReviewFailed`, so the
        // detached task has nothing left to do with the error.
        tokio::spawn(async move {
            let _ = runtime.finish_review(started).await;
        });
        Ok(handle)
    }

    /// Starts a review and awaits its findings.
    pub async fn run_review(
        self: &Arc<Self>,
        req: StartReviewRequest,
    ) -> anyhow::Result<ReviewCompletion> {
        let started = self.begin_review(req).await?;
        self.finish_review(started).await
    }

    /// Looks up a completed review by id.
    pub async fn review_record(&self, review_id: &str) -> Option<ReviewRecord> {
        self.reviews
            .lock()
            .await
            .iter()
            .rev()
            .find(|record| record.review_id == review_id)
            .cloned()
    }

    /// The most recently completed review, used when a publish request omits
    /// the review id.
    pub async fn latest_review_record(&self) -> Option<ReviewRecord> {
        self.reviews.lock().await.last().cloned()
    }

    async fn begin_review(
        self: &Arc<Self>,
        req: StartReviewRequest,
    ) -> anyhow::Result<StartedReview> {
        if let Some(turn_id) = self.active_turn_for_thread(&req.thread_id).await {
            anyhow::bail!(
                "thread {} has an active turn ({turn_id}); wait for it to finish before starting a review",
                req.thread_id
            );
        }

        // `[review].model` reserves a stronger model for reviews without
        // touching the model the requesting thread talks to.
        let model_override = match req.model_override.clone() {
            Some(model) => Some(model),
            None => self.review_config().await.model,
        };

        let workspace_root = PathBuf::from(&req.workspace);
        let vcs = self.review_vcs_provider(&workspace_root).await?;
        let request = resolve_review_request(&req.target, &workspace_root, vcs.as_ref()).await?;

        let review_thread = self
            .create_thread_with(CreateThreadRequest {
                title: Some(format!("review: {}", request.label)),
                workspace: req.workspace.clone(),
                workspace_id: None,
                root_id: Some(req.thread_id.clone()),
                provider: req.provider_override.clone(),
                model: model_override.clone(),
                selection_mode: None,
                tool_allowlist: REVIEW_TOOL_ALLOWLIST
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner: None,
            })
            .await?;

        // Subscribe before the turn starts, otherwise its first transcript
        // items can land before the receiver exists.
        let events = self.subscribe_events();

        let turn_id = self
            .start_turn(StartTurnRequest {
                thread_id: review_thread.thread_id.clone(),
                message: request.prompt.clone(),
                images: Vec::new(),
                provider_override: req.provider_override,
                model_override,
                reasoning_override: None,
                workspace: req.workspace.clone(),
                instructions: InstructionBundle {
                    system: Some(REVIEW_RUBRIC.to_string()),
                    developer: None,
                    developer_context: None,
                },
                developer_context: None,
                task_ledger_required: false,
            })
            .await?;

        let handle = ReviewHandle {
            review_id: uuid::Uuid::new_v4().to_string(),
            review_thread_id: review_thread.thread_id,
            turn_id,
        };
        self.emit(RoderEvent::ReviewStarted(ReviewStarted {
            thread_id: req.thread_id.clone(),
            turn_id: handle.turn_id.clone(),
            review_id: handle.review_id.clone(),
            review_thread_id: handle.review_thread_id.clone(),
            target: req.target,
            label: request.label.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        Ok(StartedReview {
            handle,
            request,
            parent_thread_id: req.thread_id,
            events,
        })
    }

    async fn finish_review(
        self: &Arc<Self>,
        started: StartedReview,
    ) -> anyhow::Result<ReviewCompletion> {
        let StartedReview {
            handle,
            request,
            parent_thread_id,
            mut events,
        } = started;

        let collected =
            collect_final_answer(&mut events, &handle.review_thread_id, &handle.turn_id).await;

        let text = match collected {
            Ok(text) => text,
            Err(error) => {
                self.echo_review_into_parent(&parent_thread_id, &handle.turn_id, None)
                    .await;
                self.emit(RoderEvent::ReviewFailed(ReviewFailed {
                    thread_id: parent_thread_id,
                    turn_id: Some(handle.turn_id),
                    review_id: handle.review_id,
                    review_thread_id: Some(handle.review_thread_id),
                    error: error.to_string(),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                return Err(error);
            }
        };

        let output = parse_review_output(&text);
        self.echo_review_into_parent(&parent_thread_id, &handle.turn_id, Some(&output))
            .await;
        self.remember_review(ReviewRecord {
            review_id: handle.review_id.clone(),
            parent_thread_id: parent_thread_id.clone(),
            review_thread_id: handle.review_thread_id.clone(),
            turn_id: handle.turn_id.clone(),
            workspace_root: request.workspace_root.clone(),
            request,
            output: output.clone(),
        })
        .await;
        self.emit(RoderEvent::ReviewCompleted(ReviewCompleted {
            thread_id: parent_thread_id,
            turn_id: handle.turn_id.clone(),
            review_id: handle.review_id.clone(),
            review_thread_id: handle.review_thread_id.clone(),
            output: output.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        Ok(ReviewCompletion { handle, output })
    }

    async fn review_vcs_provider(
        &self,
        workspace_root: &Path,
    ) -> anyhow::Result<Arc<dyn VcsProvider>> {
        let resolution = self
            .registry
            .version_control_resolver()
            .resolve_provider(VcsResolveRequest {
                workspace_root: workspace_root.to_path_buf(),
                preferred_provider_id: None,
            })
            .await
            .with_context(|| {
                format!(
                    "resolving a version control provider for {}",
                    workspace_root.display()
                )
            })?;
        match resolution {
            VcsProviderResolution::Available { provider, .. } => Ok(provider),
            VcsProviderResolution::Unavailable { workspace_root } => anyhow::bail!(
                "no version control provider claims {}; a review needs a repository to diff",
                workspace_root.display()
            ),
        }
    }

    /// Records the review in the requesting thread the way codex does: a
    /// synthetic `<user_action>` turn plus the rendered summary, so the parent
    /// agent can act on findings the user keeps.
    async fn echo_review_into_parent(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        output: Option<&ReviewOutput>,
    ) {
        let (user_text, assistant_text) = match output {
            Some(output) => (
                synthetic_user_action(output),
                render_review_output_text(output),
            ),
            None => (
                synthetic_user_action_interrupted(),
                REVIEW_INTERRUPTED_MESSAGE.to_string(),
            ),
        };
        // Echoing is best effort: a persistence failure must not turn a
        // successful review into a failed one.
        let _ = self
            .persist_turn_item(
                thread_id,
                turn_id,
                &TranscriptItem::UserMessage(UserMessage::text(user_text)),
            )
            .await;
        let _ = self
            .persist_turn_item(
                thread_id,
                turn_id,
                &TranscriptItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
                    phase: None,
                }),
            )
            .await;
    }

    async fn remember_review(&self, record: ReviewRecord) {
        let mut reviews = self.reviews.lock().await;
        reviews.retain(|existing| existing.review_id != record.review_id);
        reviews.push(record);
        let overflow = reviews.len().saturating_sub(REVIEW_HISTORY_LIMIT);
        if overflow > 0 {
            reviews.drain(..overflow);
        }
    }
}

/// Drains the bus until the review turn ends, returning its final answer.
///
/// The persisted `final_answer` assistant item is authoritative; streamed
/// message deltas are only a fallback for providers that end a turn without
/// one.
async fn collect_final_answer(
    events: &mut broadcast::Receiver<EventEnvelope>,
    thread_id: &ThreadId,
    turn_id: &TurnId,
) -> anyhow::Result<String> {
    let mut final_answer: Option<String> = None;
    let mut streamed = String::new();
    loop {
        let envelope = match events.recv().await {
            Ok(envelope) => envelope,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => {
                anyhow::bail!("the runtime event bus closed before the review turn finished")
            }
        };
        if envelope.thread_id.as_ref() != Some(thread_id)
            || envelope.turn_id.as_ref() != Some(turn_id)
        {
            continue;
        }
        match &envelope.event {
            RoderEvent::TranscriptItemAppended(event) => {
                if let Some(TranscriptItem::AssistantMessage(message)) = &event.item
                    && message.phase.as_deref() == Some(FINAL_ANSWER_PHASE)
                {
                    final_answer = Some(message.text.clone());
                }
            }
            RoderEvent::InferenceEventReceived(event) => {
                if let InferenceEvent::MessageDelta(delta) = &event.event {
                    streamed.push_str(&delta.text);
                }
            }
            RoderEvent::TurnCompleted(_) => break,
            RoderEvent::TurnFailed(event) => {
                anyhow::bail!("review turn failed: {}", event.error)
            }
            _ => {}
        }
    }
    Ok(final_answer.unwrap_or(streamed))
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
