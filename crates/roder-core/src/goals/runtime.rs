use super::*;

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
        let workspace = self.workspace_for_thread(&thread_id).await?;
        let turn_id = self
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: continuation_prompt(&goal),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace,
                instructions: crate::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
                service_tier_override: None,
            })
            .await?;
        Ok(Some(turn_id))
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
