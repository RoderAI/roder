use std::sync::Arc;

use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::notifications::{Notification, NotificationKind};
use roder_api::thread::{ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind};
use roder_core::Runtime;
use roder_protocol::{
    ApprovalRequestedNotification, ApprovalResolvedNotification, AutomationRunFailedNotification,
    AutomationRunNotification, AutomationRunSkippedNotification, ExternalToolCall,
    JsonRpcNotification, PlanExitRequestedNotification, PlanExitResolvedNotification,
    TeamCleanupCompletedNotification, TeamDescriptor, TeamMemberCompletedNotification,
    TeamMemberMessageDeltaNotification, TeamMemberStartedNotification,
    TeamMemberStatusChangedNotification, TeamStartedNotification, Thread,
    ThreadGoalClearedNotification, ThreadGoalUpdatedNotification, ThreadStartedNotification,
    ThreadStatus, ThreadStatusChangedNotification, ToolExecutionRequestedNotification,
    ToolExecutionResolvedNotification, Turn, TurnCompletedNotification, TurnStartedNotification,
    UserInputRequestedNotification, UserInputResolvedNotification,
    VerificationCompletedNotification, VerificationRequiredNotification,
    VerificationSkippedNotification,
};
use roder_tasks::BackgroundRunner;
use time::OffsetDateTime;
use tokio::sync::broadcast;

pub(crate) fn spawn_task_event_bridge(runtime: Arc<Runtime>, tasks: BackgroundRunner) {
    let mut task_events = tasks.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = task_events.recv().await {
            runtime.bus.emit(event);
        }
    });
}

pub(crate) fn spawn_protocol_notification_bridge(
    runtime: Arc<Runtime>,
    notifications: broadcast::Sender<JsonRpcNotification>,
) {
    let mut events = runtime.subscribe_events();
    tokio::spawn(async move {
        while let Ok(envelope) = events.recv().await {
            match crate::item_stream::item_stream_notifications_for_event(&runtime, &envelope).await
            {
                Ok(item_notifications) => {
                    for notification in item_notifications {
                        let _ = notifications.send(notification);
                    }
                }
                Err(err) => {
                    eprintln!(
                        "warning: failed to record item stream event for {}: {err:#}",
                        envelope.kind
                    );
                    if let Some(notification) =
                        item_stream_persistence_failure_notification(&envelope)
                    {
                        let _ = notifications.send(notification);
                    }
                }
            }
            for notification in protocol_notifications_for_event(&envelope.event) {
                let _ = notifications.send(notification);
            }
        }
    });
}

pub(crate) fn thread_started_notification(thread: Thread) -> JsonRpcNotification {
    protocol_notification("thread/started", ThreadStartedNotification { thread })
}

pub(crate) fn protocol_notifications_for_event(event: &RoderEvent) -> Vec<JsonRpcNotification> {
    match event {
        RoderEvent::InferenceRoutingDecision(event) => {
            vec![protocol_notification(
                "inference/routing/decision",
                roder_protocol::InferenceRoutingDecisionEvent::from(event.clone()),
            )]
        }
        RoderEvent::TurnStarted(event) => {
            let turn = Turn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "inProgress".to_string(),
                error: None,
                started_at: Some(event.timestamp.unix_timestamp()),
                completed_at: None,
                duration_ms: None,
                usage: None,
                finish_reason: None,
            };
            vec![
                protocol_notification(
                    "turn/started",
                    TurnStartedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(
                    &event.thread_id,
                    "running",
                    Some(event.turn_id.clone()),
                ),
            ]
        }
        RoderEvent::ThreadGoalUpdated(event) => vec![protocol_notification(
            "thread/goal/updated",
            ThreadGoalUpdatedNotification {
                thread_id: event.thread_id.clone(),
                goal: event.goal.clone(),
            },
        )],
        RoderEvent::ThreadGoalCleared(event) => vec![protocol_notification(
            "thread/goal/cleared",
            ThreadGoalClearedNotification {
                thread_id: event.thread_id.clone(),
            },
        )],
        RoderEvent::ApprovalRequested(event) => vec![
            protocol_notification(
                "thread/approvalRequested",
                ApprovalRequestedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    approval_id: event.approval_id.clone(),
                    tool_id: event.tool_id.clone(),
                    tool_name: event.tool_name.clone(),
                    reason: event.reason.clone(),
                },
            ),
            thread_status_notification_with_flags(
                &event.thread_id,
                "running",
                Some(event.turn_id.clone()),
                vec!["approvalRequired".to_string()],
            ),
        ],
        RoderEvent::ApprovalResolved(event) => vec![
            protocol_notification(
                "thread/approvalResolved",
                ApprovalResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    approval_id: event.approval_id.clone(),
                    tool_id: event.tool_id.clone(),
                    tool_name: event.tool_name.clone(),
                    approved: event.approved,
                },
            ),
            thread_status_notification(&event.thread_id, "running", Some(event.turn_id.clone())),
        ],
        RoderEvent::ExternalToolCallRequested(event) => vec![
            protocol_notification(
                "thread/toolExecutionRequested",
                ToolExecutionRequestedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    call: ExternalToolCall {
                        id: event.tool_id.clone(),
                        name: event.tool_name.clone(),
                        arguments: event.arguments.clone(),
                    },
                },
            ),
            thread_status_notification_with_flags(
                &event.thread_id,
                "running",
                Some(event.turn_id.clone()),
                vec!["externalToolPending".to_string()],
            ),
        ],
        RoderEvent::ExternalToolCallResolved(event) => vec![
            protocol_notification(
                "thread/toolExecutionResolved",
                ToolExecutionResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    tool_id: event.tool_id.clone(),
                    tool_name: event.tool_name.clone(),
                    outcome: event.outcome,
                    is_error: event.is_error,
                },
            ),
            thread_status_notification(&event.thread_id, "running", Some(event.turn_id.clone())),
        ],
        RoderEvent::UserInputRequested(event) => vec![
            protocol_notification(
                "thread/userInputRequested",
                UserInputRequestedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    questions: event.questions.clone(),
                },
            ),
            thread_status_notification_with_flags(
                &event.thread_id,
                "running",
                Some(event.turn_id.clone()),
                vec!["userInputRequired".to_string()],
            ),
        ],
        RoderEvent::UserInputResolved(event) => vec![
            protocol_notification(
                "thread/userInputResolved",
                UserInputResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    answers: event.answers.clone(),
                },
            ),
            thread_status_notification(&event.thread_id, "running", Some(event.turn_id.clone())),
        ],
        RoderEvent::PolicyExitPlanRequested(event) => vec![
            protocol_notification(
                "thread/planExitRequested",
                PlanExitRequestedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    target_mode: event.target_mode,
                    plan_summary: event.plan_summary.clone(),
                    next_steps: event.next_steps.clone(),
                },
            ),
            thread_status_notification_with_flags(
                &event.thread_id,
                "running",
                Some(event.turn_id.clone()),
                vec!["planExitRequired".to_string()],
            ),
        ],
        RoderEvent::PolicyExitPlanResolved(event) => vec![
            protocol_notification(
                "thread/planExitResolved",
                PlanExitResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    approved: event.approved,
                    target_mode: event.target_mode,
                    resolved_mode: event.resolved_mode,
                },
            ),
            thread_status_notification(&event.thread_id, "running", Some(event.turn_id.clone())),
        ],
        RoderEvent::TurnCompleted(event) => {
            let turn = Turn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
                usage: event.usage.clone(),
                finish_reason: event.finish_reason.clone(),
            };
            vec![
                protocol_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle", None),
            ]
        }
        RoderEvent::VerificationRequired(event) => vec![protocol_notification(
            "verification/required",
            VerificationRequiredNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                reason: event.reason.clone(),
                changed_files: event.changed_files.clone(),
                tool_evidence: event.tool_evidence.clone(),
                tests_run: event.tests_run.clone(),
                open_gaps: event.open_gaps.clone(),
            },
        )],
        RoderEvent::VerificationCompleted(event) => vec![protocol_notification(
            "verification/completed",
            VerificationCompletedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                passed: event.passed,
                changed_files: event.changed_files.clone(),
                tool_evidence: event.tool_evidence.clone(),
                tests_run: event.tests_run.clone(),
                open_gaps: event.open_gaps.clone(),
            },
        )],
        RoderEvent::VerificationSkipped(event) => vec![protocol_notification(
            "verification/skipped",
            VerificationSkippedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                reason: event.reason.clone(),
            },
        )],
        RoderEvent::AutomationStarted(event) => vec![protocol_notification(
            "automations/runStarted",
            AutomationRunNotification {
                run: event.run.clone(),
            },
        )],
        RoderEvent::AutomationCompleted(event) => vec![protocol_notification(
            "automations/runCompleted",
            AutomationRunNotification {
                run: event.run.clone(),
            },
        )],
        RoderEvent::AutomationFailed(event) => {
            let failed = protocol_notification(
                "automations/runFailed",
                AutomationRunFailedNotification {
                    run: event.run.clone(),
                    error: event.error.clone(),
                },
            );
            if automation_needs_input(&event.error) {
                vec![
                    failed,
                    protocol_notification(
                        "automations/needsInput",
                        AutomationRunFailedNotification {
                            run: event.run.clone(),
                            error: event.error.clone(),
                        },
                    ),
                ]
            } else {
                vec![failed]
            }
        }
        RoderEvent::AutomationSkipped(event) => vec![protocol_notification(
            "automations/runSkipped",
            AutomationRunSkippedNotification {
                run: event.run.clone(),
                reason: event.reason.clone(),
            },
        )],
        RoderEvent::TurnFailed(event) => {
            let turn = Turn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "failed".to_string(),
                error: Some(serde_json::json!({ "message": event.error })),
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
                usage: event.usage.clone(),
                finish_reason: None,
            };
            vec![
                protocol_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle", None),
            ]
        }
        RoderEvent::TurnInterrupted(event) => {
            let turn = Turn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "interrupted".to_string(),
                error: None,
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
                usage: None,
                finish_reason: None,
            };
            vec![
                protocol_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle", None),
            ]
        }
        RoderEvent::TeamStarted(event) => vec![protocol_notification(
            "team/started",
            TeamStartedNotification {
                team: TeamDescriptor {
                    id: event.team_id.clone(),
                    lead_thread_id: event.lead_thread_id.clone(),
                    display_mode: event.display_mode,
                    members: event.members.clone(),
                    tasks: event.tasks.clone(),
                },
            },
        )],
        RoderEvent::TeamMemberStarted(event) => vec![protocol_notification(
            "team/member/started",
            TeamMemberStartedNotification {
                team_id: event.team_id.clone(),
                member: event.member.clone(),
            },
        )],
        RoderEvent::TeamMemberStatusChanged(event) => vec![protocol_notification(
            "team/member/statusChanged",
            TeamMemberStatusChangedNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                status: event.status,
            },
        )],
        RoderEvent::TeamMemberMessageDelta(event) => vec![protocol_notification(
            "team/member/messageDelta",
            TeamMemberMessageDeltaNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                turn_id: event.turn_id.clone(),
                delta: event.delta.clone(),
            },
        )],
        RoderEvent::TeamMemberCompleted(event) => vec![protocol_notification(
            "team/member/completed",
            TeamMemberCompletedNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                turn_id: event.turn_id.clone(),
                status: event.status,
                final_message: event.final_message.clone(),
                error: event.error.clone(),
            },
        )],
        RoderEvent::TeamCleanupCompleted(event) => vec![protocol_notification(
            "team/cleanupCompleted",
            TeamCleanupCompletedNotification {
                team_id: event.team_id.clone(),
                forced: event.forced,
            },
        )],
        RoderEvent::AgentSwarmModeChanged(event) => {
            vec![protocol_notification(
                "agentSwarm/modeChanged",
                event.clone(),
            )]
        }
        RoderEvent::AgentSwarmStarted(event) => {
            vec![protocol_notification("agentSwarm/started", event.clone())]
        }
        RoderEvent::AgentSwarmProgress(event) => {
            vec![protocol_notification("agentSwarm/progress", event.clone())]
        }
        RoderEvent::AgentSwarmCompleted(event) => {
            vec![protocol_notification("agentSwarm/completed", event.clone())]
        }
        RoderEvent::SubagentTraceCreated(event) => {
            vec![protocol_notification(
                "turn/subagentTraceCreated",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceDelta(event) => {
            vec![protocol_notification(
                "turn/subagentTraceDelta",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceStatusChanged(event) => vec![protocol_notification(
            "turn/subagentTraceStatusChanged",
            event.clone(),
        )],
        RoderEvent::SubagentTraceCompleted(event) => {
            vec![protocol_notification(
                "turn/subagentTraceCompleted",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceFailed(event) => {
            vec![protocol_notification(
                "turn/subagentTraceFailed",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewCreated(event) => {
            vec![protocol_notification("plan/reviewCreated", event.clone())]
        }
        RoderEvent::PlanReviewStatusChanged(event) => {
            vec![protocol_notification(
                "plan/reviewStatusChanged",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewCommentAdded(event) => {
            vec![protocol_notification(
                "plan/reviewCommentAdded",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewRewritten(event) => {
            vec![protocol_notification("plan/reviewRewritten", event.clone())]
        }
        RoderEvent::PlanReviewApproved(event) => {
            vec![protocol_notification("plan/reviewApproved", event.clone())]
        }
        RoderEvent::PlanReviewRejected(event) => {
            vec![protocol_notification("plan/reviewRejected", event.clone())]
        }
        RoderEvent::HunkRecorded(event) => {
            vec![protocol_notification("hunk/recorded", event.clone())]
        }
        RoderEvent::WorkspaceChangeObserved(event) => {
            vec![protocol_notification(
                "workspace/changeObserved",
                event.clone(),
            )]
        }
        RoderEvent::HunkRollbackRequested(event) => {
            vec![protocol_notification(
                "hunk/rollbackRequested",
                event.clone(),
            )]
        }
        RoderEvent::HunkRollbackCompleted(event) => {
            vec![protocol_notification(
                "hunk/rollbackCompleted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportsDetected(event) => {
            vec![protocol_notification(
                "workflow/importsDetected",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportPreviewed(event) => {
            vec![protocol_notification(
                "workflow/importPreviewed",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportEnabled(event) => {
            vec![protocol_notification(
                "workflow/importEnabled",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportDisabled(event) => {
            vec![protocol_notification(
                "workflow/importDisabled",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportStale(event) => {
            vec![protocol_notification("workflow/importStale", event.clone())]
        }
        RoderEvent::WorkflowImportFailed(event) => {
            vec![protocol_notification(
                "workflow/importFailed",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowRunDrafted(event) => {
            vec![protocol_notification("workflows/drafted", event.clone())]
        }
        RoderEvent::WorkflowApprovalRequested(event) => {
            vec![protocol_notification(
                "workflows/approvalRequested",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowRunApproved(event) => {
            vec![protocol_notification("workflows/approved", event.clone())]
        }
        RoderEvent::WorkflowRunDenied(event) => {
            vec![protocol_notification("workflows/denied", event.clone())]
        }
        RoderEvent::WorkflowRunQueued(event) => {
            vec![protocol_notification("workflows/queued", event.clone())]
        }
        RoderEvent::WorkflowRunStarted(event) => {
            vec![protocol_notification("workflows/started", event.clone())]
        }
        RoderEvent::WorkflowPhaseStarted(event) => {
            vec![protocol_notification(
                "workflows/phaseStarted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowPhaseCompleted(event) => {
            vec![protocol_notification(
                "workflows/phaseCompleted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowAgentQueued(event) => {
            vec![protocol_notification(
                "workflows/agentQueued",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowAgentStarted(event) => {
            vec![protocol_notification(
                "workflows/agentStarted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowAgentCompleted(event) => {
            vec![protocol_notification(
                "workflows/agentCompleted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowAgentFailed(event) => {
            vec![protocol_notification(
                "workflows/agentFailed",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowOutputRecorded(event) => {
            vec![protocol_notification(
                "workflows/outputRecorded",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowCheckpointRecorded(event) => {
            vec![protocol_notification(
                "workflows/checkpointRecorded",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowRunPaused(event) => {
            vec![protocol_notification("workflows/paused", event.clone())]
        }
        RoderEvent::WorkflowRunResumed(event) => {
            vec![protocol_notification("workflows/resumed", event.clone())]
        }
        RoderEvent::WorkflowRunStopped(event) => {
            vec![protocol_notification("workflows/stopped", event.clone())]
        }
        RoderEvent::WorkflowRunCompleted(event) => {
            vec![protocol_notification("workflows/completed", event.clone())]
        }
        RoderEvent::WorkflowRunFailed(event) => {
            vec![protocol_notification("workflows/failed", event.clone())]
        }
        RoderEvent::RoadmapChanged(event) => {
            vec![
                protocol_notification("roadmap/changed", event.clone()),
                protocol_notification("roadmap/taskChanged", event.clone()),
                protocol_notification("roadmap/threadChanged", event.clone()),
            ]
        }
        RoderEvent::MediaArtifactCreated(event) => {
            vec![protocol_notification(
                "media/artifactCreated",
                event.clone(),
            )]
        }
        RoderEvent::MediaArtifactUpdated(event) => {
            vec![protocol_notification(
                "media/artifactUpdated",
                event.clone(),
            )]
        }
        RoderEvent::MediaArtifactDeleted(event) => {
            vec![protocol_notification(
                "media/artifactDeleted",
                event.clone(),
            )]
        }
        RoderEvent::MediaPreviewReady(event) => {
            vec![protocol_notification("media/previewReady", event.clone())]
        }
        RoderEvent::MemorySaved(event) => {
            vec![protocol_notification("memory/saved", event.clone())]
        }
        RoderEvent::MemoryUpdated(event) => {
            vec![protocol_notification("memory/updated", event.clone())]
        }
        RoderEvent::MemoryDeleted(event) => {
            vec![protocol_notification("memory/deleted", event.clone())]
        }
        RoderEvent::MemoryQueried(event) => {
            vec![protocol_notification("memory/queried", event.clone())]
        }
        RoderEvent::MemoryRecallReady(event) => {
            vec![protocol_notification("memory/recallReady", event.clone())]
        }
        RoderEvent::MemoryReembedQueued(event) => {
            vec![protocol_notification("memory/reembedQueued", event.clone())]
        }
        RoderEvent::MemoryProviderChanged(event) => {
            vec![protocol_notification(
                "memory/providerChanged",
                event.clone(),
            )]
        }
        RoderEvent::MemoryObservationRecorded(event) => {
            vec![protocol_notification(
                "memory/observationRecorded",
                event.clone(),
            )]
        }
        RoderEvent::KnowledgeSaved(event) => {
            vec![protocol_notification("knowledge/saved", event.clone())]
        }
        RoderEvent::KnowledgeUpdated(event) => {
            vec![protocol_notification("knowledge/updated", event.clone())]
        }
        RoderEvent::KnowledgeArchived(event) => {
            vec![protocol_notification("knowledge/archived", event.clone())]
        }
        RoderEvent::KnowledgeLinked(event) => {
            vec![protocol_notification("knowledge/linked", event.clone())]
        }
        _ => Vec::new(),
    }
}

fn protocol_notification<T: serde::Serialize>(method: &str, params: T) -> JsonRpcNotification {
    JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: serde_json::to_value(params).unwrap(),
    }
}

pub(crate) fn thread_item_event_notification(
    event: &ThreadItemEvent,
) -> Option<JsonRpcNotification> {
    let method = match &event.event {
        ThreadItemEventKind::ItemStarted { .. } => "item/started",
        ThreadItemEventKind::ItemCompleted { .. } => "item/completed",
        ThreadItemEventKind::ItemDelta { delta, .. } => match delta {
            ThreadItemDelta::AgentMessageText { .. } => "item/agentMessage/delta",
            ThreadItemDelta::ReasoningText { .. } => "item/reasoning/textDelta",
            ThreadItemDelta::ReasoningSummaryPartAdded { .. } => "item/reasoning/summaryPartAdded",
            ThreadItemDelta::ReasoningSummaryText { .. } => "item/reasoning/summaryTextDelta",
        },
    };
    Some(protocol_notification(
        method,
        roder_protocol::ThreadItemEvent::from(event.clone()),
    ))
}

fn automation_needs_input(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("interactive input")
        || error.contains("user input")
        || error.contains("approval")
}

fn thread_status_notification(
    thread_id: &str,
    status: &str,
    active_turn_id: Option<String>,
) -> JsonRpcNotification {
    thread_status_notification_with_flags(thread_id, status, active_turn_id, Vec::new())
}

fn thread_status_notification_with_flags(
    thread_id: &str,
    status: &str,
    active_turn_id: Option<String>,
    active_flags: Vec<String>,
) -> JsonRpcNotification {
    protocol_notification(
        "thread/status/changed",
        ThreadStatusChangedNotification {
            thread_id: thread_id.to_string(),
            status: ThreadStatus {
                kind: status.to_string(),
                active_turn_id,
                active_flags,
            },
        },
    )
}

fn item_stream_persistence_failure_notification(
    envelope: &EventEnvelope,
) -> Option<JsonRpcNotification> {
    Some(thread_status_notification_with_flags(
        envelope.thread_id.as_deref()?,
        "running",
        envelope.turn_id.clone(),
        vec!["itemPersistenceFailed".to_string()],
    ))
}

pub(crate) fn spawn_runtime_event_handlers(runtime: Arc<Runtime>, tasks: BackgroundRunner) {
    let mut events = runtime.subscribe_events();
    tokio::spawn(async move {
        while let Ok(envelope) = events.recv().await {
            let _ = tasks.handle_event(&envelope).await;
            if let Some(notification) = notification_for_event(&envelope.event) {
                for sink in &runtime.registry().notification_sinks {
                    let _ = sink.deliver(notification.clone()).await;
                }
            }
        }
    });
}

fn notification_for_event(event: &RoderEvent) -> Option<Notification> {
    let timestamp = OffsetDateTime::now_utc();
    match event {
        RoderEvent::ApprovalRequested(event) => Some(Notification {
            id: format!("approval-{}", event.approval_id),
            kind: NotificationKind::NeedsInput,
            title: "Approval needed".to_string(),
            body: Some(format!("{} is waiting for approval", event.tool_name)),
            task_id: None,
            thread_id: Some(event.thread_id.clone()),
            turn_id: Some(event.turn_id.clone()),
            timestamp,
            metadata: serde_json::json!({ "tool_id": event.tool_id }),
        }),
        RoderEvent::UserInputRequested(event) => Some(Notification {
            id: format!("user-input-{}", event.request_id),
            kind: NotificationKind::NeedsInput,
            title: "Input needed".to_string(),
            body: Some("The agent is waiting for a user choice.".to_string()),
            task_id: None,
            thread_id: Some(event.thread_id.clone()),
            turn_id: Some(event.turn_id.clone()),
            timestamp,
            metadata: serde_json::json!({ "request_id": event.request_id }),
        }),
        RoderEvent::TurnCompleted(event) => Some(Notification {
            id: format!("turn-idle-{}", event.turn_id),
            kind: NotificationKind::TurnIdle,
            title: "Turn finished".to_string(),
            body: None,
            task_id: None,
            thread_id: Some(event.thread_id.clone()),
            turn_id: Some(event.turn_id.clone()),
            timestamp,
            metadata: serde_json::json!({}),
        }),
        RoderEvent::TaskCompleted(event) => Some(Notification {
            id: format!("task-completed-{}", event.task_id),
            kind: NotificationKind::TaskCompleted,
            title: "Task completed".to_string(),
            body: Some(format!("{} finished", event.task_id)),
            task_id: Some(event.task_id.clone()),
            thread_id: event.thread_id.clone(),
            turn_id: event.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({ "exit_code": event.exit_code }),
        }),
        RoderEvent::TaskFailed(event) => Some(Notification {
            id: format!("task-failed-{}", event.task_id),
            kind: NotificationKind::TaskFailed,
            title: "Task failed".to_string(),
            body: Some(event.error.clone()),
            task_id: Some(event.task_id.clone()),
            thread_id: event.thread_id.clone(),
            turn_id: event.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({}),
        }),
        RoderEvent::AutomationStarted(event) => Some(Notification {
            id: format!("automation-started-{}", event.run.run_id),
            kind: NotificationKind::Custom("automation_started".to_string()),
            title: "Automation started".to_string(),
            body: Some(format!("{} is running", event.run.automation_id)),
            task_id: event.run.task_id.clone(),
            thread_id: event.run.thread_id.clone(),
            turn_id: event.run.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({ "automation_id": event.run.automation_id, "run_id": event.run.run_id }),
        }),
        RoderEvent::AutomationCompleted(event) => Some(Notification {
            id: format!("automation-completed-{}", event.run.run_id),
            kind: NotificationKind::Custom("automation_completed".to_string()),
            title: "Automation completed".to_string(),
            body: Some(format!("{} completed", event.run.automation_id)),
            task_id: event.run.task_id.clone(),
            thread_id: event.run.thread_id.clone(),
            turn_id: event.run.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({ "automation_id": event.run.automation_id, "run_id": event.run.run_id }),
        }),
        RoderEvent::AutomationFailed(event) if automation_needs_input(&event.error) => {
            Some(Notification {
                id: format!("automation-needs-input-{}", event.run.run_id),
                kind: NotificationKind::NeedsInput,
                title: "Automation needs input".to_string(),
                body: Some(event.error.clone()),
                task_id: event.run.task_id.clone(),
                thread_id: event.run.thread_id.clone(),
                turn_id: event.run.turn_id.clone(),
                timestamp,
                metadata: serde_json::json!({ "automation_id": event.run.automation_id, "run_id": event.run.run_id }),
            })
        }
        RoderEvent::AutomationFailed(event) => Some(Notification {
            id: format!("automation-failed-{}", event.run.run_id),
            kind: NotificationKind::Custom("automation_failed".to_string()),
            title: "Automation failed".to_string(),
            body: Some(event.error.clone()),
            task_id: event.run.task_id.clone(),
            thread_id: event.run.thread_id.clone(),
            turn_id: event.run.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({ "automation_id": event.run.automation_id, "run_id": event.run.run_id }),
        }),
        RoderEvent::AutomationSkipped(event) => Some(Notification {
            id: format!("automation-skipped-{}", event.run.run_id),
            kind: NotificationKind::Custom("automation_skipped".to_string()),
            title: "Automation skipped".to_string(),
            body: Some(event.reason.clone()),
            task_id: event.run.task_id.clone(),
            thread_id: event.run.thread_id.clone(),
            turn_id: event.run.turn_id.clone(),
            timestamp,
            metadata: serde_json::json!({ "automation_id": event.run.automation_id, "run_id": event.run.run_id }),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::automations::{
        AutomationCompleted, AutomationFailed, AutomationRunState, AutomationRunSummary,
        AutomationSkipped, AutomationStarted,
    };
    use roder_api::events::{
        InferenceEventReceived, InferenceRoutingDecisionEvent, TeamMemberCompleted,
        TeamMemberStarted, TeamStarted, TranscriptItemAppended, VerificationRequired,
    };
    use roder_api::inference::{InferenceEvent, ModelSelection, ReasoningDelta};
    use roder_api::inference_routing::InferenceRoutingDecision;
    use roder_api::notifications::NotificationKind;
    use roder_api::teams::{
        AgentTeamDisplayMode, TeamMemberDescriptor, TeamMemberRole, TeamMemberStatus,
    };
    use roder_api::thread::{
        ThreadItem, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadItemStatus,
    };
    use roder_api::transcript::{TranscriptItem, UserMessage};
    use serde_json::json;

    fn automation_run(state: AutomationRunState) -> AutomationRunSummary {
        AutomationRunSummary {
            run_id: "run-1".to_string(),
            automation_id: "automation-1".to_string(),
            occurrence_key: "automation-1:1770000000".to_string(),
            state,
            scheduled_for: OffsetDateTime::UNIX_EPOCH,
            queued_at: None,
            started_at: None,
            finished_at: None,
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            task_id: Some("task-1".to_string()),
            server_id: Some("server-1".to_string()),
            server_role: Some("desktop".to_string()),
            exit_code: None,
            error: None,
            skip_reason: None,
        }
    }

    #[test]
    fn recorded_tool_item_event_forwards_full_envelope() {
        let notifications = thread_item_event_notification(&ThreadItemEvent {
            seq: 7,
            event_id: "item-event-7".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
            event: ThreadItemEventKind::ItemCompleted {
                item: ThreadItem::ToolExecution {
                    id: "tool-1".to_string(),
                    tool_call_id: "tool-1".to_string(),
                    tool_name: "list_files".to_string(),
                    status: ThreadItemStatus::Completed,
                    input: Some(json!({ "path": ".", "shown": 3 })),
                    output: Some("src\nCargo.toml".to_string()),
                    error: None,
                },
            },
        })
        .into_iter()
        .collect::<Vec<_>>();

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, "item/completed");
        assert_eq!(notifications[0].params["seq"], 7);
        assert_eq!(notifications[0].params["eventId"], "item-event-7");
        let item = &notifications[0].params["event"]["item"];
        assert_eq!(item["type"], "toolExecution");
        assert_eq!(item["toolName"], "list_files");
        assert_eq!(item["input"]["path"], ".");
        assert_eq!(item["input"]["shown"], 3);
        assert_eq!(item["output"], "src\nCargo.toml");
        assert!(item.get("payload").is_none());
    }

    #[test]
    fn reasoning_delta_uses_dedicated_reasoning_text_notification() {
        let notifications = thread_item_event_notification(&ThreadItemEvent {
            seq: 1,
            event_id: "item-event-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
            event: ThreadItemEventKind::ItemDelta {
                item_id: "turn-1-agent-reasoning".to_string(),
                delta: ThreadItemDelta::ReasoningText {
                    delta: "Inspecting".to_string(),
                    content_index: 0,
                },
            },
        })
        .into_iter()
        .collect::<Vec<_>>();

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, "item/reasoning/textDelta");
        assert_eq!(
            notifications[0].params["event"]["itemId"],
            "turn-1-agent-reasoning"
        );
        assert_eq!(
            notifications[0].params["event"]["delta"]["delta"],
            "Inspecting"
        );
        assert_eq!(notifications[0].params["event"]["delta"]["contentIndex"], 0);
        assert!(
            notifications[0].params["event"]["delta"]
                .get("phase")
                .is_none()
        );
    }

    #[test]
    fn inference_events_do_not_directly_emit_public_item_notifications() {
        let notifications = protocol_notifications_for_event(&RoderEvent::InferenceEventReceived(
            InferenceEventReceived {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                event: InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: "Inspecting".to_string(),
                }),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));

        assert!(notifications.is_empty());
    }

    #[test]
    fn inference_routing_decision_forwards_protocol_notification() {
        let selected = ModelSelection {
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
        };
        let notifications = protocol_notifications_for_event(
            &RoderEvent::InferenceRoutingDecision(InferenceRoutingDecisionEvent {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                round_index: 1,
                default_selection: selected.clone(),
                selected_selection: selected,
                decision: InferenceRoutingDecision::selected(
                    "local",
                    ModelSelection {
                        provider: "codex".to_string(),
                        model: "gpt-5.5".to_string(),
                    },
                    "risk floor signal",
                ),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        );

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, "inference/routing/decision");
        assert_eq!(notifications[0].params["threadId"], "thread-1");
        assert_eq!(notifications[0].params["turnId"], "turn-1");
        assert_eq!(notifications[0].params["roundIndex"], 1);
        assert_eq!(notifications[0].params["decision"]["routerId"], "local");
    }

    #[test]
    fn transcript_item_appended_does_not_emit_public_item_notification() {
        let notifications = protocol_notifications_for_event(&RoderEvent::TranscriptItemAppended(
            TranscriptItemAppended {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "user_message".to_string(),
                item_index: Some(0),
                item: Some(TranscriptItem::UserMessage(UserMessage::text("hello"))),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));

        assert!(notifications.is_empty());
    }

    #[test]
    fn team_member_completed_forwards_final_message_and_error() {
        let completed = protocol_notifications_for_event(&RoderEvent::TeamMemberCompleted(
            TeamMemberCompleted {
                team_id: "team-1".to_string(),
                member_id: "member-1".to_string(),
                member_thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                status: TeamMemberStatus::Completed,
                final_message: Some("Review complete.".to_string()),
                error: None,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].method, "team/member/completed");
        assert_eq!(completed[0].params["teamId"], "team-1");
        assert_eq!(completed[0].params["memberId"], "member-1");
        assert_eq!(completed[0].params["turnId"], "turn-1");
        assert_eq!(completed[0].params["status"], "completed");
        assert_eq!(completed[0].params["finalMessage"], "Review complete.");
        assert!(completed[0].params.get("error").is_none());

        let failed = protocol_notifications_for_event(&RoderEvent::TeamMemberCompleted(
            TeamMemberCompleted {
                team_id: "team-1".to_string(),
                member_id: "member-2".to_string(),
                member_thread_id: "thread-2".to_string(),
                turn_id: Some("turn-2".to_string()),
                status: TeamMemberStatus::Failed,
                final_message: None,
                error: Some("provider request failed".to_string()),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));
        assert_eq!(failed[0].params["error"], "provider request failed");
        assert!(failed[0].params.get("finalMessage").is_none());
    }

    #[test]
    fn team_start_notifications_forward_complete_ultra_descriptors() {
        let member = TeamMemberDescriptor {
            id: "member-ultra".to_string(),
            role: TeamMemberRole::Teammate,
            name: "Reviewer".to_string(),
            task_name: Some("reviewer".to_string()),
            agent_path: Some("/root/reviewer".to_string()),
            thread_id: "thread-ultra".to_string(),
            parent_thread_id: Some("thread-root".to_string()),
            current_turn_id: Some("turn-ultra".to_string()),
            model_provider: Some("codex".to_string()),
            model: Some("gpt-5.6-terra".to_string()),
            policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
            status: TeamMemberStatus::Running,
            final_message: None,
            terminal_error: None,
            pane_id: None,
        };
        let started = protocol_notifications_for_event(&RoderEvent::TeamStarted(TeamStarted {
            team_id: "team-ultra".to_string(),
            lead_thread_id: "thread-root".to_string(),
            display_mode: AgentTeamDisplayMode::InProcess,
            members: vec![member.clone()],
            tasks: Vec::new(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        }));
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].method, "team/started");
        assert_eq!(started[0].params["team"]["id"], "team-ultra");
        assert_eq!(
            started[0].params["team"]["members"][0]["model"],
            "gpt-5.6-terra"
        );
        assert_eq!(
            started[0].params["team"]["members"][0]["agentPath"],
            "/root/reviewer"
        );

        let member_started =
            protocol_notifications_for_event(&RoderEvent::TeamMemberStarted(TeamMemberStarted {
                team_id: "team-ultra".to_string(),
                member,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(member_started.len(), 1);
        assert_eq!(member_started[0].method, "team/member/started");
        let projected = &member_started[0].params["member"];
        assert_eq!(projected["taskName"], "reviewer");
        assert_eq!(projected["agentPath"], "/root/reviewer");
        assert_eq!(projected["parentThreadId"], "thread-root");
        assert_eq!(projected["currentTurnId"], "turn-ultra");
        assert_eq!(projected["modelProvider"], "codex");
        assert_eq!(projected["model"], "gpt-5.6-terra");
        assert_eq!(projected["policyMode"], "bypass");
        assert_eq!(projected["status"], "running");
    }

    #[test]
    fn verification_required_notification_is_forwarded_to_protocol_clients() {
        let notifications = protocol_notifications_for_event(&RoderEvent::VerificationRequired(
            VerificationRequired {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                reason: "code_changes_without_verification".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                tool_evidence: vec!["write_file: wrote src/lib.rs".to_string()],
                tests_run: Vec::new(),
                open_gaps: Vec::new(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, "verification/required");
        assert_eq!(notifications[0].params["threadId"], "thread-1");
        assert_eq!(notifications[0].params["changedFiles"][0], "src/lib.rs");
        assert_eq!(
            notifications[0].params["reason"],
            "code_changes_without_verification"
        );
    }

    #[test]
    fn automations_notifications_cover_terminal_and_wait_states() {
        let started =
            protocol_notifications_for_event(&RoderEvent::AutomationStarted(AutomationStarted {
                run: automation_run(AutomationRunState::Running),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(started[0].method, "automations/runStarted");
        assert_eq!(started[0].params["run"]["automationId"], "automation-1");
        assert_eq!(started[0].params["run"]["state"], "running");

        let completed = protocol_notifications_for_event(&RoderEvent::AutomationCompleted(
            AutomationCompleted {
                run: automation_run(AutomationRunState::Completed),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));
        assert_eq!(completed[0].method, "automations/runCompleted");

        let failed =
            protocol_notifications_for_event(&RoderEvent::AutomationFailed(AutomationFailed {
                run: automation_run(AutomationRunState::Failed),
                error: "provider returned 500".to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].method, "automations/runFailed");
        assert_eq!(failed[0].params["error"], "provider returned 500");

        let needs_input =
            protocol_notifications_for_event(&RoderEvent::AutomationFailed(AutomationFailed {
                run: automation_run(AutomationRunState::Failed),
                error: "automation run blocked waiting for interactive input".to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(needs_input.len(), 2);
        assert_eq!(needs_input[0].method, "automations/runFailed");
        assert_eq!(needs_input[1].method, "automations/needsInput");

        let skipped =
            protocol_notifications_for_event(&RoderEvent::AutomationSkipped(AutomationSkipped {
                run: automation_run(AutomationRunState::Skipped),
                reason: "missed run expired".to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(skipped[0].method, "automations/runSkipped");
        assert_eq!(skipped[0].params["reason"], "missed run expired");
    }

    #[test]
    fn automations_notifications_map_to_runtime_sinks() {
        let notice = notification_for_event(&RoderEvent::AutomationFailed(AutomationFailed {
            run: automation_run(AutomationRunState::Failed),
            error: "automation run blocked waiting for interactive input".to_string(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        }))
        .expect("automation needs-input notification");

        assert_eq!(notice.kind, NotificationKind::NeedsInput);
        assert_eq!(notice.title, "Automation needs input");
        assert_eq!(notice.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(notice.metadata["automation_id"], "automation-1");
        assert_eq!(notice.metadata["run_id"], "run-1");
    }
}
