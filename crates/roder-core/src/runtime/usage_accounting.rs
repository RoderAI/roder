use super::*;

#[derive(Default)]
pub(super) struct TurnUsageAccounting {
    pub(super) runtime: Weak<Runtime>,
    completed: TokenUsage,
    current: Option<TokenUsage>,
    observed: bool,
    goal_tokens: HashMap<ThreadId, u32>,
    metadata_recorded: bool,
}

impl TurnUsageAccounting {
    fn begin_inference(&mut self) {
        if let Some(usage) = self.current.take() {
            self.completed.add_assign(&usage);
        }
    }

    fn snapshot(&self) -> Option<TokenUsage> {
        if !self.observed {
            return None;
        }
        let mut total = self.completed.clone();
        if let Some(current) = &self.current {
            total.add_assign(current);
        }
        Some(total)
    }
}

impl Runtime {
    pub(crate) async fn begin_compaction_usage(&self, turn_id: &TurnId) -> bool {
        let Some(accounting) = self.turn_usage_accounting(turn_id).await else {
            return false;
        };
        accounting.lock().await.begin_inference();
        true
    }

    pub(crate) async fn record_compaction_usage(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        usage: &TokenUsage,
    ) -> anyhow::Result<bool> {
        let Some(accounting) = self.turn_usage_accounting(turn_id).await else {
            return Ok(false);
        };
        {
            let mut accounting = accounting.lock().await;
            accounting.current = Some(usage.clone());
            accounting.observed = true;
        }
        self.sync_turn_goal_usage(thread_id, turn_id).await?;
        Ok(true)
    }

    pub(super) async fn initialize_turn_usage(self: &Arc<Self>, turn_id: &TurnId) {
        self.turn_usage.lock().await.insert(
            turn_id.clone(),
            Arc::new(Mutex::new(TurnUsageAccounting {
                runtime: Arc::downgrade(self),
                ..TurnUsageAccounting::default()
            })),
        );
    }

    async fn turn_usage_accounting(
        &self,
        turn_id: &TurnId,
    ) -> Option<Arc<Mutex<TurnUsageAccounting>>> {
        self.turn_usage.lock().await.get(turn_id).cloned()
    }

    pub(super) async fn sync_turn_goal_usage(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> anyhow::Result<()> {
        let Some(accounting) = self.turn_usage_accounting(turn_id).await else {
            return Ok(());
        };
        let Some(runtime) = accounting.lock().await.runtime.upgrade() else {
            return Ok(());
        };
        let thread_id = thread_id.clone();
        let turn_id = turn_id.clone();
        // Dropping an Abortable turn must not cancel a partially committed charge.
        tokio::spawn(async move {
            runtime
                .sync_turn_goal_usage_unshielded(&thread_id, &turn_id)
                .await
        })
        .await?
    }

    async fn sync_turn_goal_usage_unshielded(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> anyhow::Result<()> {
        let Some(accounting) = self.turn_usage_accounting(turn_id).await else {
            return Ok(());
        };
        let mut accounting = accounting.lock().await;
        let total = accounting.snapshot().map_or(0, |usage| usage.total_tokens);
        let mut recipients = vec![thread_id.clone()];
        let mut current = thread_id.clone();
        while let Some((team_id, member)) = self.teams.member_for_thread(&current).await {
            let parent = match member.parent_thread_id {
                Some(parent) => parent,
                None => match self.teams.get(&team_id).await {
                    Some(team) if team.lead_thread_id != current => team.lead_thread_id,
                    _ => break,
                },
            };
            if recipients.contains(&parent) {
                break;
            }
            recipients.push(parent.clone());
            current = parent;
        }
        for recipient in recipients {
            let charged = accounting.goal_tokens.get(&recipient).copied().unwrap_or(0);
            let delta = total.saturating_sub(charged);
            if delta == 0 {
                continue;
            }
            if &recipient == thread_id {
                self.goals
                    .record_turn_usage(&recipient, turn_id, i64::from(delta))
                    .await?;
            } else {
                self.goals
                    .record_descendant_usage(&recipient, i64::from(delta))
                    .await?;
            }
            accounting.goal_tokens.insert(recipient, total);
        }
        Ok(())
    }

    pub(super) async fn account_usage_event(&self, event: &mut RoderEvent) {
        match event {
            RoderEvent::TurnStarted(_) => return,
            RoderEvent::InferenceStarted(event) => {
                if let Some(accounting) = self.turn_usage_accounting(&event.turn_id).await {
                    accounting.lock().await.begin_inference();
                }
                return;
            }
            RoderEvent::InferenceEventReceived(event) => {
                if let InferenceEvent::Usage(usage) = &event.event
                    && let Some(accounting) = self.turn_usage_accounting(&event.turn_id).await
                {
                    let mut accounting = accounting.lock().await;
                    accounting.current = Some(usage.clone());
                    accounting.observed = true;
                }
                return;
            }
            _ => {}
        }
        let (thread_id, turn_id, usage) = match event {
            RoderEvent::TurnCompleted(event) => {
                (&event.thread_id, &event.turn_id, &mut event.usage)
            }
            RoderEvent::TurnFailed(event) => (&event.thread_id, &event.turn_id, &mut event.usage),
            RoderEvent::TurnInterrupted(event) => {
                (&event.thread_id, &event.turn_id, &mut event.usage)
            }
            _ => return,
        };
        let Some(accounting) = self.turn_usage_accounting(turn_id).await else {
            return;
        };
        let Some(runtime) = accounting.lock().await.runtime.upgrade() else {
            return;
        };
        let thread_id = thread_id.clone();
        let turn_id = turn_id.clone();
        let provided_usage = usage.clone();
        let finalized = tokio::spawn(async move {
            runtime
                .finalize_turn_usage(&thread_id, &turn_id, provided_usage)
                .await
        })
        .await;
        match finalized {
            Ok(observed) => *usage = observed,
            Err(error) => eprintln!("failed to finalize terminal usage: {error}"),
        }
    }

    async fn finalize_turn_usage(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provided: Option<TokenUsage>,
    ) -> Option<TokenUsage> {
        let accounting = self.turn_usage_accounting(turn_id).await?;
        if let Err(error) = self
            .sync_turn_goal_usage_unshielded(thread_id, turn_id)
            .await
        {
            eprintln!("failed to finalize turn token budget: {error}");
        }
        let mut accounting = accounting.lock().await;
        let usage = accounting.snapshot().or(provided);
        if !accounting.metadata_recorded
            && let Some(usage) = &usage
        {
            match self
                .record_terminal_usage_metadata(thread_id, turn_id, usage)
                .await
            {
                Ok(()) => accounting.metadata_recorded = true,
                Err(error) => eprintln!("failed to persist terminal turn usage: {error}"),
            }
        }
        usage
    }

    async fn record_terminal_usage_metadata(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        usage: &TokenUsage,
    ) -> anyhow::Result<()> {
        let _guard = self.thread_usage_metadata_lock.lock().await;
        let Some(store) = &self.thread_store else {
            return Ok(());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            return Ok(());
        };
        let Some(mut metadata) = snapshot.metadata else {
            return Ok(());
        };
        if !metadata
            .usage
            .get_or_insert_with(ThreadUsageMetadata::default)
            .add_turn_token_usage(turn_id, usage)
        {
            return Ok(());
        }
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;
        Ok(())
    }
}
