use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use roder_api::events::{RoderEvent, ThreadId};
use roder_api::goals::{
    ThreadGoal, ThreadGoalCleared, ThreadGoalController, ThreadGoalPatch, ThreadGoalStatus,
    ThreadGoalUpdated, validate_thread_goal_budget, validate_thread_goal_objective,
};
use roder_api::inference::InstructionBundle;
use roder_api::thread::ThreadStore;
use time::{Duration, OffsetDateTime};

mod accounting;
mod prompts;
mod runtime;
#[cfg(test)]
mod tests;

use prompts::{continuation_prompt, objective_updated_prompt};
use tokio::sync::Mutex;

use crate::bus::EventBus;
use crate::runtime::{Runtime, StartTurnRequest};

const GOAL_STATE_FILE: &str = "goal.json";

#[derive(Debug, Default)]
struct GoalCache {
    goals: HashMap<ThreadId, Option<ThreadGoal>>,
    turns: HashMap<String, accounting::GoalTurnProgress>,
    empty_turns: HashMap<ThreadId, u8>,
}

#[derive(Clone)]
pub struct RuntimeGoalController {
    bus: EventBus,
    thread_store: Option<Arc<dyn ThreadStore>>,
    thread_root: Option<PathBuf>,
    cache: Arc<Mutex<GoalCache>>,
    mutation: Arc<Mutex<()>>,
}

impl RuntimeGoalController {
    pub fn new(bus: EventBus, thread_store: Option<Arc<dyn ThreadStore>>) -> Self {
        let thread_root = thread_store
            .as_ref()
            .and_then(|store| store.local_thread_root());
        Self {
            bus,
            thread_store,
            thread_root,
            cache: Arc::new(Mutex::new(GoalCache::default())),
            mutation: Arc::new(Mutex::new(())),
        }
    }

    pub async fn apply_goal_instructions(
        &self,
        thread_id: &ThreadId,
        mut instructions: InstructionBundle,
    ) -> anyhow::Result<InstructionBundle> {
        let Some(goal) = self.get_thread_goal(thread_id).await? else {
            return Ok(instructions);
        };
        if !matches!(
            goal.status,
            ThreadGoalStatus::Active | ThreadGoalStatus::BudgetLimited
        ) {
            return Ok(instructions);
        }
        let addition = if goal.status == ThreadGoalStatus::BudgetLimited {
            if !self
                .cache
                .lock()
                .await
                .turns
                .values()
                .any(|progress| progress.matches_goal(&goal))
            {
                return Ok(instructions);
            }
            prompts::budget_limit_prompt(&goal)
        } else {
            continuation_prompt(&goal)
        };
        instructions.developer = Some(match instructions.developer {
            Some(existing) if !existing.trim().is_empty() => format!("{existing}\n\n{addition}"),
            _ => addition,
        });
        Ok(instructions)
    }

    /// Account work explicitly attributed to the active goal (including compaction).
    pub async fn account_turn_usage(
        &self,
        thread_id: &ThreadId,
        tokens_used: i64,
        elapsed: Duration,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let _guard = self.mutation.lock().await;
        let Some(mut goal) = self.load_goal(thread_id).await? else {
            return Ok(None);
        };
        if goal.status != ThreadGoalStatus::Active {
            return Ok(Some(goal));
        }
        goal.tokens_used = goal.tokens_used.saturating_add(tokens_used.max(0));
        goal.time_used_seconds = goal
            .time_used_seconds
            .saturating_add(elapsed.whole_seconds().max(0));
        enforce_budget(&mut goal);
        goal.updated_at = OffsetDateTime::now_utc();
        self.store_goal(goal.clone()).await?;
        self.emit_goal_updated(goal.clone()).await;
        Ok(Some(goal))
    }

    pub(crate) async fn is_continuation(
        &self,
        thread_id: &ThreadId,
        message: &str,
    ) -> anyhow::Result<bool> {
        Ok(self
            .load_goal(thread_id)
            .await?
            .is_some_and(|goal| message == continuation_prompt(&goal)))
    }

    pub async fn active_goal(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadGoal>> {
        Ok(self
            .get_thread_goal(thread_id)
            .await?
            .filter(|goal| goal.status == ThreadGoalStatus::Active))
    }

    async fn load_goal(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadGoal>> {
        let mut cache = self.cache.lock().await;
        if let Some(goal) = cache.goals.get(thread_id) {
            return Ok(goal.clone());
        }
        let goal = match self.goal_path(thread_id) {
            Some(path) if path.exists() => {
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("read goal state {}", path.display()))?;
                Some(
                    serde_json::from_slice::<ThreadGoal>(&bytes)
                        .with_context(|| format!("parse goal state {}", path.display()))?,
                )
            }
            _ => None,
        };
        cache.goals.insert(thread_id.clone(), goal.clone());
        Ok(goal)
    }

    async fn store_goal(&self, goal: ThreadGoal) -> anyhow::Result<()> {
        if let Some(path) = self.goal_path(&goal.thread_id) {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("create goal directory {}", parent.display()))?;
            }
            let bytes = serde_json::to_vec_pretty(&goal).context("serialize goal state")?;
            let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            tokio::fs::write(&temporary, bytes)
                .await
                .with_context(|| format!("write goal state {}", temporary.display()))?;
            tokio::fs::rename(&temporary, &path)
                .await
                .with_context(|| format!("commit goal state {}", path.display()))?;
        }
        self.cache
            .lock()
            .await
            .goals
            .insert(goal.thread_id.clone(), Some(goal));
        Ok(())
    }

    async fn remove_goal(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let existed = self.load_goal(thread_id).await?.is_some();
        if let Some(path) = self.goal_path(thread_id)
            && path.exists()
        {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("remove goal state {}", path.display()))?;
        }
        self.cache
            .lock()
            .await
            .goals
            .insert(thread_id.clone(), None);
        Ok(existed)
    }

    fn goal_path(&self, thread_id: &ThreadId) -> Option<PathBuf> {
        self.thread_root
            .as_ref()
            .map(|root| root.join(thread_id).join(GOAL_STATE_FILE))
    }

    async fn emit_goal_updated(&self, goal: ThreadGoal) {
        let event = RoderEvent::ThreadGoalUpdated(ThreadGoalUpdated {
            thread_id: goal.thread_id.clone(),
            goal,
            timestamp: OffsetDateTime::now_utc(),
        });
        self.emit_goal_event(event).await;
    }

    async fn emit_goal_cleared(&self, thread_id: ThreadId) {
        let event = RoderEvent::ThreadGoalCleared(ThreadGoalCleared {
            thread_id,
            timestamp: OffsetDateTime::now_utc(),
        });
        self.emit_goal_event(event).await;
    }

    async fn emit_goal_event(&self, event: RoderEvent) {
        let envelope = self.bus.emit(event);
        if let (Some(store), Some(thread_id)) = (&self.thread_store, envelope.thread_id.as_ref()) {
            let _ = store.append_event(thread_id, &envelope).await;
        }
    }
}

#[async_trait::async_trait]
impl ThreadGoalController for RuntimeGoalController {
    async fn get_thread_goal(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadGoal>> {
        let _guard = self.mutation.lock().await;
        self.flush_thread_progress(thread_id).await?;
        self.load_goal(thread_id).await
    }

    async fn create_thread_goal(
        &self,
        thread_id: &ThreadId,
        objective: String,
        token_budget: Option<i64>,
    ) -> anyhow::Result<ThreadGoal> {
        let objective = objective.trim().to_string();
        validate_thread_goal_objective(&objective)?;
        validate_thread_goal_budget(token_budget)?;
        let _guard = self.mutation.lock().await;
        if self
            .load_goal(thread_id)
            .await?
            .is_some_and(|goal| goal.status != ThreadGoalStatus::Complete)
        {
            anyhow::bail!(
                "cannot create a new goal because this thread has an unfinished goal; complete the existing goal first"
            );
        }
        let now = OffsetDateTime::now_utc();
        let goal = ThreadGoal {
            thread_id: thread_id.clone(),
            objective,
            status: ThreadGoalStatus::Active,
            token_budget,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: now,
            updated_at: now,
        };
        self.store_goal(goal.clone()).await?;
        self.attach_goal_to_turns(&goal).await;
        self.emit_goal_updated(goal.clone()).await;
        Ok(goal)
    }

    async fn set_thread_goal(
        &self,
        thread_id: &ThreadId,
        patch: ThreadGoalPatch,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        if let Some(objective) = patch.objective.as_deref() {
            validate_thread_goal_objective(objective)?;
        }
        if let Some(token_budget) = patch.token_budget {
            validate_thread_goal_budget(token_budget)?;
        }
        let _guard = self.mutation.lock().await;
        self.flush_thread_progress(thread_id).await?;
        let now = OffsetDateTime::now_utc();
        let mut goal = match self.load_goal(thread_id).await? {
            Some(goal) => goal,
            None => {
                let Some(objective) = patch.objective.as_ref() else {
                    return Ok(None);
                };
                ThreadGoal {
                    thread_id: thread_id.clone(),
                    objective: objective.trim().to_string(),
                    status: ThreadGoalStatus::Active,
                    token_budget: None,
                    tokens_used: 0,
                    time_used_seconds: 0,
                    created_at: now,
                    updated_at: now,
                }
            }
        };
        if let Some(objective) = patch.objective {
            goal.objective = objective.trim().to_string();
        }
        if let Some(status) = patch.status {
            // Exhausted budgets take precedence over a requested pause or blocked state.
            if goal.status != ThreadGoalStatus::BudgetLimited
                || !matches!(status, ThreadGoalStatus::Paused | ThreadGoalStatus::Blocked)
            {
                goal.status = status;
            }
        }
        if let Some(token_budget) = patch.token_budget {
            goal.token_budget = token_budget;
        }
        enforce_budget(&mut goal);
        goal.updated_at = OffsetDateTime::now_utc();
        self.store_goal(goal.clone()).await?;
        self.attach_goal_to_turns(&goal).await;
        self.emit_goal_updated(goal.clone()).await;
        Ok(Some(goal))
    }

    async fn clear_thread_goal(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let _guard = self.mutation.lock().await;
        let cleared = self.remove_goal(thread_id).await?;
        self.detach_thread_goal(thread_id).await;
        if cleared {
            self.emit_goal_cleared(thread_id.clone()).await;
        }
        Ok(cleared)
    }
}

fn enforce_budget(goal: &mut ThreadGoal) {
    if goal.status == ThreadGoalStatus::Active
        && goal
            .token_budget
            .is_some_and(|budget| goal.tokens_used >= budget)
    {
        goal.status = ThreadGoalStatus::BudgetLimited;
    }
}

pub(crate) fn status_after_error(error: &anyhow::Error) -> ThreadGoalStatus {
    use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};
    if error
        .downcast_ref::<ProviderFailure>()
        .is_some_and(|failure| {
            matches!(
                failure.kind,
                ProviderFailureKind::UsageLimit | ProviderFailureKind::QuotaExceeded
            )
        })
    {
        ThreadGoalStatus::UsageLimited
    } else {
        ThreadGoalStatus::Blocked
    }
}
