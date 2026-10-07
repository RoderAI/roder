use super::*;

impl RuntimeGoalController {
    pub(crate) async fn continuation_request(
        &self,
        thread_id: &ThreadId,
    ) -> Option<StartTurnRequest> {
        self.cache
            .lock()
            .await
            .continuation_requests
            .get(thread_id)
            .cloned()
    }

    pub(crate) async fn inherit_thread_goal_snapshot(
        &self,
        source: &ThreadId,
        target: &ThreadId,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let _guard = self.mutation.lock().await;
        self.flush_thread_progress(source).await?;
        let Some(mut goal) = self.load_goal(source).await? else {
            return Ok(None);
        };
        validate_thread_goal_objective(&goal.objective)?;
        goal.thread_id = target.clone();
        self.store_goal(goal.clone()).await?;
        Ok(Some(goal))
    }

    /// Keep goal mutations excluded until idle turn admission is committed.
    pub(crate) async fn admit_continuation(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<(tokio::sync::OwnedMutexGuard<()>, ThreadGoal)>> {
        let guard = self.mutation.clone().lock_owned().await;
        self.flush_thread_progress(thread_id).await?;
        Ok(self
            .load_goal(thread_id)
            .await?
            .filter(|goal| goal.status.is_active())
            .map(|goal| (guard, goal)))
    }

    /// Keep the admitted turn's configuration for automatic continuation. A
    /// later explicit turn replaces it; user input and attachments are never
    /// replayed, and this authority context is never persisted to disk.
    pub(crate) async fn remember_turn_options(&self, request: &StartTurnRequest) {
        self.cache.lock().await.continuation_requests.insert(
            request.thread_id.clone(),
            StartTurnRequest {
                thread_id: request.thread_id.clone(),
                message: String::new(),
                images: Vec::new(),
                provider_override: request.provider_override.clone(),
                model_override: request.model_override.clone(),
                reasoning_override: request.reasoning_override.clone(),
                workspace: request.workspace.clone(),
                instructions: request.instructions.clone(),
                developer_context: request.developer_context.clone(),
                task_ledger_required: request.task_ledger_required,
                service_tier_override: request.service_tier_override.clone(),
            },
        );
    }
}

impl Runtime {
    pub async fn thread_goal_get(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        self.goals.get_thread_goal(thread_id).await
    }

    pub async fn thread_goal_set(
        &self,
        thread_id: &ThreadId,
        patch: ThreadGoalPatch,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        self.goals.set_thread_goal(thread_id, patch).await
    }

    pub async fn thread_goal_clear(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let cleared = self.goals.clear_thread_goal(thread_id).await?;
        if cleared && let Some(turn_id) = self.active_turn_for_thread(thread_id).await {
            let _ = self.steer_turn(thread_id.clone(), turn_id,
                "The user cleared the thread goal. Stop autonomous goal work; respond to any remaining explicit user instructions.".into(), Vec::new()).await;
        }
        Ok(cleared)
    }

    pub async fn apply_external_goal_set_effects(
        self: &Arc<Self>,
        previous_goal: Option<ThreadGoal>,
        goal: Option<ThreadGoal>,
    ) -> anyhow::Result<Option<ThreadId>> {
        let Some(goal) = goal else {
            return Ok(None);
        };
        if goal.status != ThreadGoalStatus::Active {
            if let Some(turn_id) = self.active_turn_for_thread(&goal.thread_id).await {
                let message = if goal.status == ThreadGoalStatus::BudgetLimited {
                    super::prompts::budget_limit_prompt(&goal)
                } else {
                    format!(
                        "The user set the goal status to {}. Stop autonomous goal work and report this status.",
                        goal.status.as_str()
                    )
                };
                self.steer_turn(goal.thread_id.clone(), turn_id.clone(), message, Vec::new())
                    .await?;
                return Ok(Some(turn_id));
            }
            return Ok(None);
        }

        let objective_changed = previous_goal
            .as_ref()
            .is_none_or(|previous| previous.objective != goal.objective);
        let resumed = previous_goal
            .as_ref()
            .is_some_and(|previous| !previous.status.is_active());
        if (objective_changed || resumed)
            && let Some(turn_id) = self.active_turn_for_thread(&goal.thread_id).await
        {
            self.steer_turn(
                goal.thread_id.clone(),
                turn_id.clone(),
                if objective_changed { objective_updated_prompt(&goal) } else {
                    format!("The user resumed the goal. The earlier pause request is revoked. Start a fresh blocked audit and continue pursuing the full objective.\n\n{}", continuation_prompt(&goal))
                },
                Vec::new(),
            )
            .await?;
            return Ok(Some(turn_id));
        }

        self.continue_active_goal_if_idle(goal.thread_id.clone())
            .await
    }

    pub async fn continue_active_goal_if_idle(
        self: &Arc<Self>,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<ThreadId>> {
        if self.has_active_turn_for_thread(&thread_id).await {
            return Ok(None);
        }
        let Some(goal) = self.goals.active_goal(&thread_id).await? else {
            return Ok(None);
        };
        let previous = self.goals.continuation_request(&thread_id).await;
        let mut request = match previous {
            Some(request) => request,
            None => StartTurnRequest {
                thread_id: thread_id.clone(),
                message: String::new(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: self.workspace_for_thread(&thread_id).await?,
                instructions: crate::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
                service_tier_override: None,
            },
        };
        request.message = continuation_prompt(&goal);
        self.start_goal_turn_if_idle(request).await
    }

    pub(crate) async fn continue_active_goal_after_turn(
        self: &Arc<Self>,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<ThreadId>> {
        self.continue_active_goal_if_idle(thread_id).await
    }
}

impl Runtime {
    pub(crate) async fn record_goal_token_usage(
        &self,
        thread_id: &ThreadId,
        turn_id: &str,
        tokens: i64,
    ) -> anyhow::Result<()> {
        self.goals
            .record_turn_usage(thread_id, turn_id, tokens)
            .await?;
        // Delegated work consumes the ancestor goal's budget, even while the
        // lead waits for its children. Charge each ancestor once.
        let mut current = thread_id.clone();
        let mut visited = std::collections::HashSet::from([current.clone()]);
        while let Some((team_id, member)) = self.teams.member_for_thread(&current).await {
            let parent = match member.parent_thread_id {
                Some(parent) => parent,
                None => match self.teams.get(&team_id).await {
                    Some(team) if team.lead_thread_id != current => team.lead_thread_id,
                    _ => break,
                },
            };
            if !visited.insert(parent.clone()) {
                break;
            }
            self.goals.record_descendant_usage(&parent, tokens).await?;
            current = parent;
        }
        Ok(())
    }
}
