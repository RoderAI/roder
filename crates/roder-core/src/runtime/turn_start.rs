use super::*;

impl Runtime {
    pub fn start_turn(
        self: &Arc<Self>,
        req: StartTurnRequest,
    ) -> BoxFuture<'_, anyhow::Result<TurnId>> {
        Box::pin(async move {
            self.start_turn_admitted(req, false)
                .await?
                .ok_or_else(|| anyhow::anyhow!("explicit turn was not admitted"))
        })
    }

    pub(crate) fn start_goal_turn_if_idle(
        self: &Arc<Self>,
        req: StartTurnRequest,
    ) -> BoxFuture<'_, anyhow::Result<Option<TurnId>>> {
        self.start_turn_admitted(req, true)
    }

    fn start_turn_admitted(
        self: &Arc<Self>,
        mut req: StartTurnRequest,
        goal_only: bool,
    ) -> BoxFuture<'_, anyhow::Result<Option<TurnId>>> {
        Box::pin(async move {
            let _thread_admission = self.thread_admission(&req.thread_id).await;
            let turn_admission = self.turn_admission.lock().await;
            let _goal_admission = if goal_only {
                if self.has_active_turn_for_thread(&req.thread_id).await {
                    return Ok(None);
                }
                let Some((guard, goal)) = self.goals.admit_continuation(&req.thread_id).await?
                else {
                    return Ok(None);
                };
                if let Some(previous) = self.goals.continuation_request(&req.thread_id).await {
                    req = previous;
                }
                req.message = crate::goals::continuation_prompt(&goal);
                Some(guard)
            } else {
                None
            };
            self.ensure_execution_authority()?;
            anyhow::ensure!(
                self.accepting_turns.load(Ordering::Acquire),
                "runtime is quiescing and cannot accept new turns"
            );
            req.workspace = validate_thread_workspace(&req.workspace)?;
            let team_member = self.teams.member_for_thread(&req.thread_id).await;
            let cfg = self.config.read().await.clone();
            let provider = req
                .provider_override
                .clone()
                .unwrap_or_else(|| cfg.default_provider.clone());
            self.engine_for(&provider)?;
            let turn_id = uuid::Uuid::new_v4().to_string();
            let mut initial_mailbox_ack = None;
            if let Some((team_id, member)) = &team_member {
                let pending = self
                    .teams
                    .reserve_pending_mailbox_messages(team_id, &member.id, &turn_id)
                    .await?;
                if !pending.is_empty()
                    && let Some(team) = self.read_team(team_id).await
                {
                    let mailbox = format_mailbox_messages(&team, &pending);
                    req.message = if req.message.trim().is_empty() {
                        mailbox
                    } else {
                        format!("{mailbox}\n\n[Direct task input]\n{}", req.message)
                    };
                    initial_mailbox_ack = Some(MailboxDeliveryAck {
                        team_id: team_id.clone(),
                        message_ids: pending.iter().map(|message| message.id.clone()).collect(),
                    });
                }
            }
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let drain = Arc::new(TurnDrainHandle {
                thread_id: req.thread_id.clone(),
                interrupt_requested: AtomicBool::new(false),
                interrupt_reason: Mutex::new(None),
                completed: AtomicBool::new(false),
                completed_notify: Notify::new(),
            });
            let (steer_changed, steering) = tokio::sync::watch::channel(0);
            let active = ActiveTurnHandle {
                thread_id: req.thread_id.clone(),
                abort: abort_handle,
                steers: Arc::new(Mutex::new(Vec::new())),
                steer_changed,
                drain,
            };
            self.active_turns
                .write()
                .await
                .insert(turn_id.clone(), active);
            self.record_turn_lifecycle(
                req.thread_id.clone(),
                turn_id.clone(),
                TurnLifecycleState::Running,
                TurnCleanupState::NotRequested,
                None,
            )
            .await;
            self.active_turn_contexts.write().await.insert(
                turn_id.clone(),
                InheritedTurnContext {
                    workspace: req.workspace.clone(),
                    instructions: req.instructions.clone(),
                    developer_context: req.developer_context.clone(),
                },
            );
            if let Some((team_id, member)) = team_member {
                let updated = match self
                    .teams
                    .update_member(&team_id, &member.id, |member| {
                        member.current_turn_id = Some(turn_id.clone());
                        member.status = TeamMemberStatus::Running;
                        member.final_message = None;
                        member.terminal_error = None;
                    })
                    .await
                {
                    Ok(updated) => updated,
                    Err(error) => {
                        self.active_turns.write().await.remove(&turn_id);
                        self.active_turn_contexts.write().await.remove(&turn_id);
                        self.teams
                            .release_mailbox_reservations_for_turn(&turn_id)
                            .await;
                        return Err(error);
                    }
                };
                if let Some(member) = updated
                    .members
                    .into_iter()
                    .find(|candidate| candidate.id == member.id)
                {
                    self.emit(RoderEvent::TeamMemberStatusChanged(
                        TeamMemberStatusChanged {
                            team_id,
                            member_id: member.id,
                            member_thread_id: member.thread_id,
                            status: TeamMemberStatus::Running,
                            timestamp: OffsetDateTime::now_utc(),
                        },
                    ))
                    .await;
                }
            }
            self.goals.remember_turn_options(&req).await;
            let runtime = Arc::clone(self);
            let turn_req = req;
            let thread_id_for_task = turn_req.thread_id.clone();
            let turn_id_for_task = turn_id.clone();
            tokio::spawn(async move {
                let result = Abortable::new(
                    runtime.run_turn(
                        turn_req,
                        turn_id_for_task.clone(),
                        initial_mailbox_ack,
                        steering,
                        goal_only,
                    ),
                    abort_registration,
                )
                .await;
                /*
                 * A failed sibling in a parallel tool batch drops in-flight external tool
                 * futures (`try_join_all` in `route_tool_calls`), stranding their
                 * `pending_external_tool_calls` entries. Sweep before reporting the turn
                 * outcome so every `thread/toolExecutionRequested` gets a terminal
                 * resolution; on clean completion the map holds nothing for this turn.
                 */
                runtime
                    .cancel_pending_external_tool_calls_for_turn(&turn_id_for_task)
                    .await;
                let completed = matches!(&result, Ok(Ok(TurnRunOutcome::Completed)));
                let stopped_status = match &result {
                    Ok(Err(error)) => Some(crate::goals::status_after_error(error)),
                    Ok(Ok(TurnRunOutcome::Stopped)) => {
                        Some(roder_api::goals::ThreadGoalStatus::Blocked)
                    }
                    Err(_) => Some(roder_api::goals::ThreadGoalStatus::Paused),
                    _ => None,
                };
                if let Err(error) = runtime
                    .goals
                    .finish_turn(&thread_id_for_task, &turn_id_for_task, stopped_status)
                    .await
                {
                    eprintln!("failed to finalize goal accounting: {error}");
                }
                match &result {
                    Ok(Err(err)) => {
                        let (cleanup, ownership) =
                            runtime.await_provider_turn_cleanup(&turn_id_for_task).await;
                        // This wrapper owns terminal failure emission for returned errors.
                        runtime
                            .emit(RoderEvent::TurnFailed(TurnFailed {
                                thread_id: thread_id_for_task.clone(),
                                turn_id: turn_id_for_task.clone(),
                                error: err.to_string(),
                                error_kind: err
                                    .downcast_ref::<roder_api::provider_error::ProviderFailure>()
                                    .map(|failure| failure.kind.retry_cause().to_string()),
                                usage: None,
                                timestamp: OffsetDateTime::now_utc(),
                            }))
                            .await;
                        runtime
                            .record_turn_lifecycle_with_ownership(
                                thread_id_for_task.clone(),
                                turn_id_for_task.clone(),
                                TurnLifecycleState::Failed,
                                cleanup,
                                Some(TurnLifecycleReason::ProviderFailure),
                                ownership,
                            )
                            .await;
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Failed,
                                None,
                                Some(err.to_string()),
                            )
                            .await;
                    }
                    Ok(Ok(TurnRunOutcome::Stopped)) => {
                        let (cleanup, ownership) =
                            runtime.await_provider_turn_cleanup(&turn_id_for_task).await;
                        runtime
                            .record_turn_lifecycle_with_ownership(
                                thread_id_for_task.clone(),
                                turn_id_for_task.clone(),
                                TurnLifecycleState::Failed,
                                cleanup,
                                Some(TurnLifecycleReason::ProviderFailure),
                                ownership,
                            )
                            .await;
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Failed,
                                None,
                                Some("turn stopped before completion".to_string()),
                            )
                            .await;
                    }
                    Err(_) => {
                        let reason = if let Some(handle) = runtime
                            .turn_drains
                            .read()
                            .await
                            .get(&turn_id_for_task)
                            .cloned()
                        {
                            handle
                                .interrupt_reason
                                .lock()
                                .await
                                .unwrap_or(TurnLifecycleReason::RuntimeFailure)
                        } else {
                            TurnLifecycleReason::RuntimeFailure
                        };
                        let (cleanup, ownership) =
                            runtime.await_provider_turn_cleanup(&turn_id_for_task).await;
                        runtime
                            .record_turn_lifecycle_with_ownership(
                                thread_id_for_task.clone(),
                                turn_id_for_task.clone(),
                                TurnLifecycleState::Interrupted,
                                cleanup,
                                Some(reason),
                                ownership,
                            )
                            .await;
                        runtime
                            .emit(RoderEvent::TurnInterrupted(TurnInterrupted {
                                thread_id: thread_id_for_task.clone(),
                                turn_id: turn_id_for_task.clone(),
                                timestamp: OffsetDateTime::now_utc(),
                            }))
                            .await;
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Interrupted,
                                None,
                                None,
                            )
                            .await;
                    }
                    Ok(Ok(TurnRunOutcome::Completed)) => {}
                }
                runtime
                    .teams
                    .release_mailbox_reservations_for_turn(&turn_id_for_task)
                    .await;
                runtime.active_turns.write().await.remove(&turn_id_for_task);
                if let Some(drain) = runtime.turn_drains.write().await.remove(&turn_id_for_task) {
                    drain.completed.store(true, Ordering::Release);
                    drain.completed_notify.notify_waiters();
                }
                runtime
                    .active_turn_selections
                    .write()
                    .await
                    .remove(&turn_id_for_task);
                runtime
                    .active_turn_contexts
                    .write()
                    .await
                    .remove(&turn_id_for_task);
                if !completed {
                    // Completion paths above consume registered provider cleanup
                    // handles. A setup failure before an engine can stream has no
                    // such handle; this is a harmless final defensive sweep.
                    let _ = runtime.provider_turn_cleanups.lock().map(|mut cleanups| {
                        cleanups.remove(&turn_id_for_task);
                    });
                }
                runtime.active_turns_changed.notify_waiters();
                if completed {
                    let _ = runtime
                        .continue_active_goal_after_turn(thread_id_for_task)
                        .await;
                }
            });
            drop(turn_admission);
            Ok(Some(turn_id))
        })
    }
}
