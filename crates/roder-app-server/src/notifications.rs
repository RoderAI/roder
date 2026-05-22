use std::sync::Arc;

use roder_api::conversation::tool_display_payload;
use roder_api::events::RoderEvent;
use roder_api::inference::InferenceEvent;
use roder_api::notifications::{Notification, NotificationKind};
use roder_core::Runtime;
use roder_protocol::{
    AgentMessageDeltaNotification, ApprovalRequestedNotification, ApprovalResolvedNotification,
    AutomationRunFailedNotification, AutomationRunNotification, AutomationRunSkippedNotification,
    DesktopItem, DesktopThread, DesktopThreadStatus, DesktopTurn, ItemCompletedNotification,
    ItemStartedNotification, JsonRpcNotification, PlanExitRequestedNotification,
    PlanExitResolvedNotification, TeamCleanupCompletedNotification,
    TeamMemberCompletedNotification, TeamMemberMessageDeltaNotification,
    TeamMemberStartedNotification, TeamMemberStatusChangedNotification,
    ThreadGoalClearedNotification, ThreadGoalUpdatedNotification, ThreadStartedNotification,
    ThreadStatusChangedNotification, TurnCompletedNotification, TurnStartedNotification,
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

pub(crate) fn spawn_desktop_notification_bridge(
    runtime: Arc<Runtime>,
    notifications: broadcast::Sender<JsonRpcNotification>,
) {
    let mut events = runtime.subscribe_events();
    tokio::spawn(async move {
        while let Ok(envelope) = events.recv().await {
            for notification in desktop_notifications_for_event(&envelope.event) {
                let _ = notifications.send(notification);
            }
        }
    });
}

pub(crate) fn thread_started_notification(thread: DesktopThread) -> JsonRpcNotification {
    desktop_notification("thread/started", ThreadStartedNotification { thread })
}

pub(crate) fn desktop_notifications_for_event(event: &RoderEvent) -> Vec<JsonRpcNotification> {
    match event {
        RoderEvent::TurnStarted(event) => {
            let turn = DesktopTurn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "inProgress".to_string(),
                error: None,
                started_at: Some(event.timestamp.unix_timestamp()),
                completed_at: None,
                duration_ms: None,
            };
            vec![
                desktop_notification(
                    "turn/started",
                    TurnStartedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "running"),
            ]
        }
        RoderEvent::InferenceEventReceived(event) => match &event.event {
            InferenceEvent::MessageDelta(delta) => vec![desktop_notification(
                "item/agentMessage/delta",
                AgentMessageDeltaNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    item_id: agent_message_item_id(&event.turn_id, delta.phase.as_deref()),
                    delta: delta.text.clone(),
                    phase: delta.phase.clone(),
                },
            )],
            InferenceEvent::ReasoningDelta(delta) => vec![desktop_notification(
                "item/agentMessage/delta",
                AgentMessageDeltaNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    item_id: agent_message_item_id(&event.turn_id, Some("reasoning")),
                    delta: delta.text.clone(),
                    phase: Some("reasoning".to_string()),
                },
            )],
            InferenceEvent::ToolCallStarted(call) => vec![desktop_notification(
                "item/started",
                ItemStartedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    item: tool_call_item(&call.id, Some(&call.name), None, "inProgress", None),
                },
            )],
            InferenceEvent::HostedToolCallStarted(call) => vec![desktop_notification(
                "item/started",
                ItemStartedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    item: tool_call_item(&call.id, Some(&call.name), None, "inProgress", None),
                },
            )],
            InferenceEvent::ToolCallCompleted(call) => vec![
                desktop_notification(
                    "item/started",
                    ItemStartedNotification {
                        thread_id: event.thread_id.clone(),
                        turn_id: event.turn_id.clone(),
                        item: tool_call_item(
                            &call.id,
                            Some(&call.name),
                            parsed_tool_display_payload(&call.name, &call.arguments),
                            "inProgress",
                            None,
                        ),
                    },
                ),
                desktop_notification(
                    "item/completed",
                    ItemCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn_id: event.turn_id.clone(),
                        item: tool_call_item(
                            &call.id,
                            Some(&call.name),
                            parsed_tool_display_payload(&call.name, &call.arguments),
                            "completed",
                            None,
                        ),
                    },
                ),
            ],
            InferenceEvent::HostedToolCallCompleted(call) => vec![
                desktop_notification(
                    "item/started",
                    ItemStartedNotification {
                        thread_id: event.thread_id.clone(),
                        turn_id: event.turn_id.clone(),
                        item: tool_call_item(
                            &call.id,
                            Some(&call.name),
                            parsed_tool_display_payload(&call.name, &call.arguments),
                            "inProgress",
                            None,
                        ),
                    },
                ),
                desktop_notification(
                    "item/completed",
                    ItemCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn_id: event.turn_id.clone(),
                        item: tool_call_item(
                            &call.id,
                            Some(&call.name),
                            parsed_tool_display_payload(&call.name, &call.arguments),
                            "completed",
                            None,
                        ),
                    },
                ),
            ],
            _ => Vec::new(),
        },
        RoderEvent::ToolCallRequested(event) => vec![desktop_notification(
            "item/started",
            ItemStartedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                item: tool_call_item(
                    &event.tool_id,
                    Some(&event.tool_name),
                    event.display_payload.clone(),
                    "inProgress",
                    None,
                ),
            },
        )],
        RoderEvent::ToolCallStarted(event) => vec![desktop_notification(
            "item/started",
            ItemStartedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                item: tool_call_item(
                    &event.tool_id,
                    event.tool_name.as_deref(),
                    event.display_payload.clone(),
                    "inProgress",
                    None,
                ),
            },
        )],
        RoderEvent::ToolCallCompleted(event) => vec![desktop_notification(
            "item/completed",
            ItemCompletedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                item: tool_result_item(
                    &event.tool_id,
                    event.tool_name.as_deref(),
                    event.display_payload.clone(),
                    event.output.clone(),
                    if event.is_error {
                        "failed"
                    } else {
                        "completed"
                    },
                ),
            },
        )],
        RoderEvent::ThreadGoalUpdated(event) => vec![desktop_notification(
            "thread/goal/updated",
            ThreadGoalUpdatedNotification {
                thread_id: event.thread_id.clone(),
                goal: event.goal.clone(),
            },
        )],
        RoderEvent::ThreadGoalCleared(event) => vec![desktop_notification(
            "thread/goal/cleared",
            ThreadGoalClearedNotification {
                thread_id: event.thread_id.clone(),
            },
        )],
        RoderEvent::ApprovalRequested(event) => vec![
            desktop_notification(
                "session/approvalRequested",
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
                vec!["approvalRequired".to_string()],
            ),
        ],
        RoderEvent::ApprovalResolved(event) => vec![
            desktop_notification(
                "session/approvalResolved",
                ApprovalResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    approval_id: event.approval_id.clone(),
                    tool_id: event.tool_id.clone(),
                    tool_name: event.tool_name.clone(),
                    approved: event.approved,
                },
            ),
            thread_status_notification(&event.thread_id, "running"),
        ],
        RoderEvent::UserInputRequested(event) => vec![
            desktop_notification(
                "session/userInputRequested",
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
                vec!["userInputRequired".to_string()],
            ),
        ],
        RoderEvent::UserInputResolved(event) => vec![
            desktop_notification(
                "session/userInputResolved",
                UserInputResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    answers: event.answers.clone(),
                },
            ),
            thread_status_notification(&event.thread_id, "running"),
        ],
        RoderEvent::PolicyExitPlanRequested(event) => vec![
            desktop_notification(
                "session/planExitRequested",
                PlanExitRequestedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    target_mode: event.target_mode,
                    plan_summary: event.plan_summary.clone(),
                },
            ),
            thread_status_notification_with_flags(
                &event.thread_id,
                "running",
                vec!["planExitRequired".to_string()],
            ),
        ],
        RoderEvent::PolicyExitPlanResolved(event) => vec![
            desktop_notification(
                "session/planExitResolved",
                PlanExitResolvedNotification {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    request_id: event.request_id.clone(),
                    approved: event.approved,
                    target_mode: event.target_mode,
                    resolved_mode: event.resolved_mode,
                },
            ),
            thread_status_notification(&event.thread_id, "running"),
        ],
        RoderEvent::TurnCompleted(event) => {
            let turn = DesktopTurn {
                id: event.turn_id.clone(),
                items: vec![DesktopItem {
                    id: agent_message_item_id(&event.turn_id, None),
                    kind: "agentMessage".to_string(),
                    text: None,
                    status: Some("completed".to_string()),
                    phase: None,
                    tool_name: None,
                    tool_call_id: None,
                    payload: None,
                }],
                items_view: "default".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
            };
            vec![
                desktop_notification(
                    "item/completed",
                    ItemCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn_id: event.turn_id.clone(),
                        item: turn.items[0].clone(),
                    },
                ),
                desktop_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle"),
            ]
        }
        RoderEvent::VerificationRequired(event) => vec![desktop_notification(
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
        RoderEvent::VerificationCompleted(event) => vec![desktop_notification(
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
        RoderEvent::VerificationSkipped(event) => vec![desktop_notification(
            "verification/skipped",
            VerificationSkippedNotification {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                reason: event.reason.clone(),
            },
        )],
        RoderEvent::AutomationStarted(event) => vec![desktop_notification(
            "automations/runStarted",
            AutomationRunNotification {
                run: event.run.clone(),
            },
        )],
        RoderEvent::AutomationCompleted(event) => vec![desktop_notification(
            "automations/runCompleted",
            AutomationRunNotification {
                run: event.run.clone(),
            },
        )],
        RoderEvent::AutomationFailed(event) => {
            let failed = desktop_notification(
                "automations/runFailed",
                AutomationRunFailedNotification {
                    run: event.run.clone(),
                    error: event.error.clone(),
                },
            );
            if automation_needs_input(&event.error) {
                vec![
                    failed,
                    desktop_notification(
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
        RoderEvent::AutomationSkipped(event) => vec![desktop_notification(
            "automations/runSkipped",
            AutomationRunSkippedNotification {
                run: event.run.clone(),
                reason: event.reason.clone(),
            },
        )],
        RoderEvent::TurnFailed(event) => {
            let turn = DesktopTurn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "failed".to_string(),
                error: Some(serde_json::json!({ "message": event.error })),
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
            };
            vec![
                desktop_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle"),
            ]
        }
        RoderEvent::TurnInterrupted(event) => {
            let turn = DesktopTurn {
                id: event.turn_id.clone(),
                items: Vec::new(),
                items_view: "default".to_string(),
                status: "interrupted".to_string(),
                error: None,
                started_at: None,
                completed_at: Some(event.timestamp.unix_timestamp()),
                duration_ms: None,
            };
            vec![
                desktop_notification(
                    "turn/completed",
                    TurnCompletedNotification {
                        thread_id: event.thread_id.clone(),
                        turn,
                    },
                ),
                thread_status_notification(&event.thread_id, "idle"),
            ]
        }
        RoderEvent::TeamMemberStarted(event) => vec![desktop_notification(
            "team/member/started",
            TeamMemberStartedNotification {
                team_id: event.team_id.clone(),
                member: roder_api::teams::TeamMemberDescriptor {
                    id: event.member_id.clone(),
                    role: event.role,
                    name: event.name.clone(),
                    thread_id: event.member_thread_id.clone(),
                    current_turn_id: None,
                    model_provider: None,
                    model: None,
                    policy_mode: roder_api::policy_mode::PolicyMode::Default,
                    status: roder_api::teams::TeamMemberStatus::Idle,
                    pane_id: None,
                },
            },
        )],
        RoderEvent::TeamMemberStatusChanged(event) => vec![desktop_notification(
            "team/member/statusChanged",
            TeamMemberStatusChangedNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                status: event.status,
            },
        )],
        RoderEvent::TeamMemberMessageDelta(event) => vec![desktop_notification(
            "team/member/messageDelta",
            TeamMemberMessageDeltaNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                turn_id: event.turn_id.clone(),
                delta: event.delta.clone(),
            },
        )],
        RoderEvent::TeamMemberCompleted(event) => vec![desktop_notification(
            "team/member/completed",
            TeamMemberCompletedNotification {
                team_id: event.team_id.clone(),
                member_id: event.member_id.clone(),
                turn_id: event.turn_id.clone(),
                status: event.status,
            },
        )],
        RoderEvent::TeamCleanupCompleted(event) => vec![desktop_notification(
            "team/cleanupCompleted",
            TeamCleanupCompletedNotification {
                team_id: event.team_id.clone(),
                forced: event.forced,
            },
        )],
        RoderEvent::SubagentTraceCreated(event) => {
            vec![desktop_notification(
                "turn/subagentTraceCreated",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceDelta(event) => {
            vec![desktop_notification(
                "turn/subagentTraceDelta",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceStatusChanged(event) => vec![desktop_notification(
            "turn/subagentTraceStatusChanged",
            event.clone(),
        )],
        RoderEvent::SubagentTraceCompleted(event) => {
            vec![desktop_notification(
                "turn/subagentTraceCompleted",
                event.clone(),
            )]
        }
        RoderEvent::SubagentTraceFailed(event) => {
            vec![desktop_notification(
                "turn/subagentTraceFailed",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewCreated(event) => {
            vec![desktop_notification("plan/reviewCreated", event.clone())]
        }
        RoderEvent::PlanReviewStatusChanged(event) => {
            vec![desktop_notification(
                "plan/reviewStatusChanged",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewCommentAdded(event) => {
            vec![desktop_notification(
                "plan/reviewCommentAdded",
                event.clone(),
            )]
        }
        RoderEvent::PlanReviewRewritten(event) => {
            vec![desktop_notification("plan/reviewRewritten", event.clone())]
        }
        RoderEvent::PlanReviewApproved(event) => {
            vec![desktop_notification("plan/reviewApproved", event.clone())]
        }
        RoderEvent::PlanReviewRejected(event) => {
            vec![desktop_notification("plan/reviewRejected", event.clone())]
        }
        RoderEvent::HunkRecorded(event) => {
            vec![desktop_notification("hunk/recorded", event.clone())]
        }
        RoderEvent::HunkRollbackRequested(event) => {
            vec![desktop_notification(
                "hunk/rollbackRequested",
                event.clone(),
            )]
        }
        RoderEvent::HunkRollbackCompleted(event) => {
            vec![desktop_notification(
                "hunk/rollbackCompleted",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportsDetected(event) => {
            vec![desktop_notification(
                "workflow/importsDetected",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportPreviewed(event) => {
            vec![desktop_notification(
                "workflow/importPreviewed",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportEnabled(event) => {
            vec![desktop_notification(
                "workflow/importEnabled",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportDisabled(event) => {
            vec![desktop_notification(
                "workflow/importDisabled",
                event.clone(),
            )]
        }
        RoderEvent::WorkflowImportStale(event) => {
            vec![desktop_notification("workflow/importStale", event.clone())]
        }
        RoderEvent::WorkflowImportFailed(event) => {
            vec![desktop_notification("workflow/importFailed", event.clone())]
        }
        RoderEvent::RoadmapChanged(event) => {
            vec![
                desktop_notification("roadmap/changed", event.clone()),
                desktop_notification("roadmap/taskChanged", event.clone()),
                desktop_notification("roadmap/threadChanged", event.clone()),
            ]
        }
        RoderEvent::MediaArtifactCreated(event) => {
            vec![desktop_notification("media/artifactCreated", event.clone())]
        }
        RoderEvent::MediaArtifactUpdated(event) => {
            vec![desktop_notification("media/artifactUpdated", event.clone())]
        }
        RoderEvent::MediaArtifactDeleted(event) => {
            vec![desktop_notification("media/artifactDeleted", event.clone())]
        }
        RoderEvent::MediaPreviewReady(event) => {
            vec![desktop_notification("media/previewReady", event.clone())]
        }
        RoderEvent::MemorySaved(event) => vec![desktop_notification("memory/saved", event.clone())],
        RoderEvent::MemoryUpdated(event) => {
            vec![desktop_notification("memory/updated", event.clone())]
        }
        RoderEvent::MemoryDeleted(event) => {
            vec![desktop_notification("memory/deleted", event.clone())]
        }
        RoderEvent::MemoryQueried(event) => {
            vec![desktop_notification("memory/queried", event.clone())]
        }
        RoderEvent::MemoryRecallReady(event) => {
            vec![desktop_notification("memory/recallReady", event.clone())]
        }
        RoderEvent::MemoryReembedQueued(event) => {
            vec![desktop_notification("memory/reembedQueued", event.clone())]
        }
        RoderEvent::MemoryProviderChanged(event) => {
            vec![desktop_notification(
                "memory/providerChanged",
                event.clone(),
            )]
        }
        RoderEvent::MemoryObservationRecorded(event) => {
            vec![desktop_notification(
                "memory/observationRecorded",
                event.clone(),
            )]
        }
        _ => Vec::new(),
    }
}

fn desktop_notification<T: serde::Serialize>(method: &str, params: T) -> JsonRpcNotification {
    JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: serde_json::to_value(params).unwrap(),
    }
}

fn automation_needs_input(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("interactive input")
        || error.contains("user input")
        || error.contains("approval")
}

fn thread_status_notification(thread_id: &str, status: &str) -> JsonRpcNotification {
    thread_status_notification_with_flags(thread_id, status, Vec::new())
}

fn thread_status_notification_with_flags(
    thread_id: &str,
    status: &str,
    active_flags: Vec<String>,
) -> JsonRpcNotification {
    desktop_notification(
        "thread/status/changed",
        ThreadStatusChangedNotification {
            thread_id: thread_id.to_string(),
            status: DesktopThreadStatus {
                kind: status.to_string(),
                active_flags,
            },
        },
    )
}

fn agent_message_item_id(turn_id: &str, phase: Option<&str>) -> String {
    format!("{}-agent-{}", turn_id, phase.unwrap_or("final_answer"))
}

fn tool_call_item(
    tool_id: &str,
    tool_name: Option<&str>,
    payload: Option<serde_json::Value>,
    status: &str,
    text: Option<String>,
) -> DesktopItem {
    DesktopItem {
        id: tool_id.to_string(),
        kind: tool_name
            .map(|name| format!("tool.{name}"))
            .unwrap_or_else(|| "toolCall".to_string()),
        text,
        status: Some(status.to_string()),
        phase: None,
        tool_name: tool_name.map(str::to_string),
        tool_call_id: Some(tool_id.to_string()),
        payload,
    }
}

fn tool_result_item(
    tool_id: &str,
    tool_name: Option<&str>,
    payload: Option<serde_json::Value>,
    output: Option<String>,
    status: &str,
) -> DesktopItem {
    DesktopItem {
        id: format!("{tool_id}-result"),
        kind: "toolMessage".to_string(),
        text: output,
        status: Some(status.to_string()),
        phase: None,
        tool_name: tool_name.map(str::to_string),
        tool_call_id: Some(tool_id.to_string()),
        payload,
    }
}

fn parsed_tool_display_payload(tool_name: &str, arguments: &str) -> Option<serde_json::Value> {
    let arguments = serde_json::from_str(arguments).ok();
    tool_display_payload(Some(tool_name), arguments.as_ref(), None)
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
    use roder_api::events::{InferenceEventReceived, ToolCallCompleted, VerificationRequired};
    use roder_api::inference::{HostedToolCallCompleted, InferenceEvent};
    use roder_api::notifications::NotificationKind;
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
    fn completed_tool_notification_carries_display_payload() {
        let notifications =
            desktop_notifications_for_event(&RoderEvent::ToolCallCompleted(ToolCallCompleted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                tool_id: "tool-1".to_string(),
                tool_name: Some("list_files".to_string()),
                display_payload: Some(json!({ "path": ".", "shown": 3 })),
                is_error: false,
                output: Some("src\nCargo.toml".to_string()),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));

        assert_eq!(notifications.len(), 1);
        let item = &notifications[0].params["item"];
        assert_eq!(item["type"], "toolMessage");
        assert_eq!(item["toolName"], "list_files");
        assert_eq!(item["payload"]["path"], ".");
        assert_eq!(item["payload"]["shown"], 3);
        assert!(item["payload"].get("input").is_none());
        assert!(item["payload"].get("arguments").is_none());
    }

    #[test]
    fn hosted_tool_call_notification_completes_tool_item() {
        let notifications = desktop_notifications_for_event(&RoderEvent::InferenceEventReceived(
            InferenceEventReceived {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                event: InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted {
                    id: "ws-1".to_string(),
                    name: "web_search".to_string(),
                    arguments: r#"{"action":"search","query":"pandelis zembashis"}"#.to_string(),
                }),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));

        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].method, "item/started");
        assert_eq!(notifications[1].method, "item/completed");
        let item = &notifications[1].params["item"];
        assert_eq!(item["type"], "tool.web_search");
        assert_eq!(item["status"], "completed");
        assert_eq!(item["toolName"], "web_search");
        assert_eq!(item["payload"]["query"], "pandelis zembashis");
    }

    #[test]
    fn verification_required_notification_is_forwarded_to_desktop_clients() {
        let notifications = desktop_notifications_for_event(&RoderEvent::VerificationRequired(
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
            desktop_notifications_for_event(&RoderEvent::AutomationStarted(AutomationStarted {
                run: automation_run(AutomationRunState::Running),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(started[0].method, "automations/runStarted");
        assert_eq!(started[0].params["run"]["automationId"], "automation-1");
        assert_eq!(started[0].params["run"]["state"], "running");

        let completed = desktop_notifications_for_event(&RoderEvent::AutomationCompleted(
            AutomationCompleted {
                run: automation_run(AutomationRunState::Completed),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));
        assert_eq!(completed[0].method, "automations/runCompleted");

        let failed =
            desktop_notifications_for_event(&RoderEvent::AutomationFailed(AutomationFailed {
                run: automation_run(AutomationRunState::Failed),
                error: "provider returned 500".to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].method, "automations/runFailed");
        assert_eq!(failed[0].params["error"], "provider returned 500");

        let needs_input =
            desktop_notifications_for_event(&RoderEvent::AutomationFailed(AutomationFailed {
                run: automation_run(AutomationRunState::Failed),
                error: "automation run blocked waiting for interactive input".to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }));
        assert_eq!(needs_input.len(), 2);
        assert_eq!(needs_input[0].method, "automations/runFailed");
        assert_eq!(needs_input[1].method, "automations/needsInput");

        let skipped =
            desktop_notifications_for_event(&RoderEvent::AutomationSkipped(AutomationSkipped {
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
