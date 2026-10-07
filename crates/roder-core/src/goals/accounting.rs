use std::time::Instant;

use super::*;

#[derive(Debug)]
pub(super) struct GoalTurnProgress {
    thread_id: ThreadId,
    goal_created_at: Option<OffsetDateTime>,
    started: Instant,
    seconds_accounted: i64,
    automatic: bool,
    activity: bool,
}

impl GoalTurnProgress {
    pub(super) fn matches_goal(&self, goal: &ThreadGoal) -> bool {
        self.thread_id == goal.thread_id && self.goal_created_at == Some(goal.created_at)
    }
}

impl RuntimeGoalController {
    pub(crate) async fn begin_turn(
        &self,
        thread_id: &ThreadId,
        turn_id: &str,
        automatic: bool,
    ) -> anyhow::Result<()> {
        let _guard = self.mutation.lock().await;
        let goal = self.load_goal(thread_id).await?;
        let goal_created_at = goal
            .filter(|goal| goal.status.is_active())
            .map(|goal| goal.created_at);
        let mut cache = self.cache.lock().await;
        if !automatic {
            cache.empty_turns.remove(thread_id);
        }
        cache.turns.insert(
            turn_id.to_string(),
            GoalTurnProgress {
                thread_id: thread_id.clone(),
                goal_created_at,
                started: Instant::now(),
                seconds_accounted: 0,
                automatic,
                activity: false,
            },
        );
        Ok(())
    }

    pub(crate) async fn record_activity(&self, turn_id: &str) {
        if let Some(progress) = self.cache.lock().await.turns.get_mut(turn_id) {
            progress.activity = true;
        }
    }

    pub(crate) async fn record_turn_usage(
        &self,
        thread_id: &ThreadId,
        turn_id: &str,
        tokens: i64,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let _guard = self.mutation.lock().await;
        self.flush_progress(thread_id, turn_id, tokens).await
    }

    pub(crate) async fn finish_turn(
        &self,
        thread_id: &ThreadId,
        turn_id: &str,
        stopped_status: Option<ThreadGoalStatus>,
    ) -> anyhow::Result<()> {
        let _guard = self.mutation.lock().await;
        self.flush_progress(thread_id, turn_id, 0).await?;
        let mut goal = self.load_goal(thread_id).await?;
        let progress = self.cache.lock().await.turns.remove(turn_id);
        let Some(progress) = progress else {
            return Ok(());
        };
        let Some(ref mut goal) = goal else {
            return Ok(());
        };
        if progress.goal_created_at != Some(goal.created_at) {
            return Ok(());
        }
        let status = if stopped_status.is_some() {
            stopped_status
        } else if progress.automatic && !progress.activity {
            let mut cache = self.cache.lock().await;
            let count = cache.empty_turns.entry(thread_id.clone()).or_default();
            *count = count.saturating_add(1);
            (*count >= 3).then_some(ThreadGoalStatus::Blocked)
        } else {
            self.cache.lock().await.empty_turns.remove(thread_id);
            None
        };
        if let Some(status) = status
            && (goal.status.is_active()
                || (goal.status == ThreadGoalStatus::BudgetLimited
                    && status == ThreadGoalStatus::UsageLimited))
        {
            goal.status = status;
            goal.updated_at = OffsetDateTime::now_utc();
            self.store_goal(goal.clone()).await?;
            self.emit_goal_updated(goal.clone()).await;
        }
        Ok(())
    }

    pub(super) async fn flush_thread_progress(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let turns: Vec<_> = self
            .cache
            .lock()
            .await
            .turns
            .iter()
            .filter(|(_, progress)| &progress.thread_id == thread_id)
            .map(|(turn_id, _)| turn_id.clone())
            .collect();
        for turn_id in turns {
            self.flush_progress(thread_id, &turn_id, 0).await?;
        }
        Ok(())
    }

    async fn flush_progress(
        &self,
        thread_id: &ThreadId,
        turn_id: &str,
        tokens: i64,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let Some(mut goal) = self.load_goal(thread_id).await? else {
            return Ok(None);
        };
        let cache = self.cache.lock().await;
        let Some(progress) = cache.turns.get(turn_id) else {
            return Ok(Some(goal));
        };
        if progress.goal_created_at != Some(goal.created_at)
            || !matches!(
                goal.status,
                ThreadGoalStatus::Active | ThreadGoalStatus::BudgetLimited
            )
        {
            return Ok(Some(goal));
        }
        let seconds = progress.started.elapsed().as_secs().min(i64::MAX as u64) as i64;
        let delta = seconds.saturating_sub(progress.seconds_accounted);
        drop(cache);
        if tokens <= 0 && delta == 0 {
            return Ok(Some(goal));
        }
        goal.tokens_used = goal.tokens_used.saturating_add(tokens.max(0));
        goal.time_used_seconds = goal.time_used_seconds.saturating_add(delta);
        enforce_budget(&mut goal);
        goal.updated_at = OffsetDateTime::now_utc();
        self.store_goal(goal.clone()).await?;
        if let Some(progress) = self.cache.lock().await.turns.get_mut(turn_id) {
            progress.seconds_accounted = seconds;
        }
        self.emit_goal_updated(goal.clone()).await;
        Ok(Some(goal))
    }

    pub(super) async fn attach_goal_to_turns(&self, goal: &ThreadGoal) {
        let mut cache = self.cache.lock().await;
        cache.empty_turns.remove(&goal.thread_id);
        for progress in cache
            .turns
            .values_mut()
            .filter(|p| p.thread_id == goal.thread_id)
        {
            if goal.status.is_active() {
                if progress.goal_created_at != Some(goal.created_at) {
                    progress.started = Instant::now();
                    progress.seconds_accounted = 0;
                }
                progress.goal_created_at = Some(goal.created_at);
            } else if goal.status != ThreadGoalStatus::BudgetLimited {
                progress.goal_created_at = None;
            }
        }
    }

    pub(super) async fn detach_thread_goal(&self, thread_id: &ThreadId) {
        let mut cache = self.cache.lock().await;
        cache.empty_turns.remove(thread_id);
        for progress in cache
            .turns
            .values_mut()
            .filter(|p| &p.thread_id == thread_id)
        {
            progress.goal_created_at = None;
        }
    }
}

impl RuntimeGoalController {
    pub(crate) async fn record_descendant_usage(
        &self,
        thread_id: &ThreadId,
        tokens: i64,
    ) -> anyhow::Result<()> {
        let _guard = self.mutation.lock().await;
        let Some(mut goal) = self.load_goal(thread_id).await? else {
            return Ok(());
        };
        let in_flight = self.cache.lock().await.turns.values().any(|progress| {
            progress.thread_id == *thread_id && progress.goal_created_at == Some(goal.created_at)
        });
        if !(goal.status.is_active() || goal.status == ThreadGoalStatus::BudgetLimited && in_flight)
        {
            return Ok(());
        }
        goal.tokens_used = goal.tokens_used.saturating_add(tokens.max(0));
        enforce_budget(&mut goal);
        goal.updated_at = OffsetDateTime::now_utc();
        self.store_goal(goal.clone()).await?;
        self.emit_goal_updated(goal).await;
        Ok(())
    }
}
