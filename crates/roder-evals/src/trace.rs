use roder_api::events::{RoderEvent, ThreadId, TurnId};
use roder_api::inference::InferenceEvent;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalRun {
    pub suite_id: String,
    pub run_id: String,
    pub provider: String,
    pub model: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrajectory {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub events: Vec<EvalTrajectoryEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrajectoryEvent {
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub event_type: String,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<EvalTokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_policy_phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_policy_reasoning: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalTokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_prompt_tokens: u32,
    #[serde(default)]
    pub cache_creation_prompt_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalMetric {
    pub name: String,
    pub kind: EvalMetricKind,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalMetricKind {
    Outcome,
    Count,
    Duration,
    Tokens,
    Bytes,
    Flag,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalOutcome {
    Pass,
    Fail,
    Timeout,
    HarnessError,
    VerifierUncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalFailureClass {
    Model,
    ToolSchema,
    Runtime,
    Environment,
    Provider,
    Verifier,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    pub run: EvalRun,
    pub outcome: EvalOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<EvalFailureClass>,
    pub trajectory: EvalTrajectory,
    #[serde(default)]
    pub metrics: Vec<EvalMetric>,
}

impl EvalTrajectory {
    pub fn from_events(
        thread_id: impl Into<ThreadId>,
        turn_id: impl Into<TurnId>,
        events: &[RoderEvent],
    ) -> Self {
        let thread_id = thread_id.into();
        let turn_id = turn_id.into();
        let events = events
            .iter()
            .filter_map(EvalTrajectoryEvent::from_event)
            .collect();
        Self {
            thread_id,
            turn_id,
            events,
        }
    }
}

impl EvalTrajectoryEvent {
    pub fn from_event(event: &RoderEvent) -> Option<Self> {
        match event {
            RoderEvent::TurnStarted(e) => {
                let mut event = Self::basic("turn_started", &e.thread_id, &e.turn_id, e.timestamp);
                event.runtime_profile = Some(e.runtime_profile.as_str().to_string());
                Some(event)
            }
            RoderEvent::InferenceStarted(e) => {
                let mut event =
                    Self::basic("inference_started", &e.thread_id, &e.turn_id, e.timestamp);
                if let Some(decision) = &e.speed_policy {
                    event.speed_policy_phase = Some(decision.phase.as_str().to_string());
                    event.speed_policy_reasoning = decision
                        .applied_reasoning
                        .clone()
                        .or_else(|| Some(decision.desired_reasoning.clone()));
                }
                Some(event)
            }
            RoderEvent::ContextAssemblyCompleted(e) => Some(Self::basic(
                "context_assembly_completed",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::ContextEntrypointCandidatesInjected(e) => Some(Self::basic(
                "entrypoint_candidates_injected",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::ContextCompactionStarted(e) => Some(Self::basic(
                "context_compaction_started",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::ContextCompactionRecorded(e) => Some(Self::basic(
                "context_compaction_recorded",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::RetrievalRoutePlanned(e) => Some(Self::basic(
                "retrieval_route_planned",
                &e.plan.thread_id,
                &e.plan.turn_id,
                e.plan.timestamp,
            )),
            RoderEvent::RetrievalRouteAccepted(e) => {
                let mut event = Self::basic(
                    "retrieval_route_accepted",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.tool_name = Some(e.tool.clone());
                Some(event)
            }
            RoderEvent::RetrievalRouteIgnored(e) => {
                let mut event = Self::basic(
                    "retrieval_route_ignored",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.tool_name = Some(e.chosen_tool.clone());
                Some(event)
            }
            RoderEvent::RetrievalRouteFailed(e) => {
                let mut event = Self::basic(
                    "retrieval_route_failed",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.tool_name = Some(e.tool.clone());
                event.is_error = true;
                Some(event)
            }
            RoderEvent::RetrievalResultUsed(e) => {
                let mut event = Self::basic(
                    "retrieval_result_used",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.tool_name = Some(e.outcome.tool.clone());
                event.is_error = !matches!(
                    e.outcome.outcome,
                    roder_api::retrieval::RetrievalOutcomeKind::Useful
                );
                Some(event)
            }
            RoderEvent::RetrievalDiscoveryItemPromoted(e) => Some(Self::basic(
                "retrieval_discovery_item_promoted",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::RetrievalPromotionSkipped(e) => {
                let mut event = Self::basic(
                    "retrieval_promotion_skipped",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.is_error = true;
                Some(event)
            }
            RoderEvent::InferenceEventReceived(e) => {
                let mut event =
                    Self::basic("inference_event", &e.thread_id, &e.turn_id, e.timestamp);
                if let InferenceEvent::Usage(usage) = &e.event {
                    event.token_usage = Some(EvalTokenUsage {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: usage.total_tokens,
                        cached_prompt_tokens: usage.cached_prompt_tokens,
                        cache_creation_prompt_tokens: usage.cache_creation_prompt_tokens,
                        cache_hit_rate: usage.cache_hit_rate,
                    });
                }
                Some(event)
            }
            RoderEvent::ToolCallRequested(e) => {
                let mut event =
                    Self::basic("tool_call_requested", &e.thread_id, &e.turn_id, e.timestamp);
                event.tool_id = Some(e.tool_id.clone());
                event.tool_name = Some(e.tool_name.clone());
                Some(event)
            }
            RoderEvent::ToolCallStarted(e) => {
                let mut event =
                    Self::basic("tool_call_started", &e.thread_id, &e.turn_id, e.timestamp);
                event.tool_id = Some(e.tool_id.clone());
                event.tool_name = e.tool_name.clone();
                Some(event)
            }
            RoderEvent::ToolCallCompleted(e) => {
                let mut event =
                    Self::basic("tool_call_completed", &e.thread_id, &e.turn_id, e.timestamp);
                event.tool_id = Some(e.tool_id.clone());
                event.tool_name = e.tool_name.clone();
                event.is_error = e.is_error;
                Some(event)
            }
            RoderEvent::ToolOutputTruncated(e) => {
                let mut event = Self::basic(
                    "tool_output_truncated",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.tool_id = Some(e.tool_id.clone());
                event.tool_name = e.tool_name.clone();
                Some(event)
            }
            RoderEvent::TaskLedgerUpdated(e) => Some(Self::basic(
                "task_ledger_updated",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::VerificationRequired(e) => Some(Self::basic(
                "verification_required",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::VerificationCompleted(e) => {
                let mut event = Self::basic(
                    "verification_completed",
                    &e.thread_id,
                    &e.turn_id,
                    e.timestamp,
                );
                event.is_error = !e.passed;
                Some(event)
            }
            RoderEvent::VerificationSkipped(e) => Some(Self::basic(
                "verification_skipped",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::ReliabilityFailureRecorded(e) => {
                let mut event = Self::basic(
                    "reliability_failure",
                    &e.context.thread_id,
                    &e.context.turn_id,
                    e.timestamp,
                );
                event.tool_id = e.context.tool_id.clone();
                event.tool_name = e.context.tool_name.clone();
                event.is_error = true;
                Some(event)
            }
            RoderEvent::ReliabilityRetryRecorded(e) => Some(Self::basic(
                "reliability_retry",
                &e.context.thread_id,
                &e.context.turn_id,
                e.timestamp,
            )),
            RoderEvent::ReliabilityLimitRecorded(e) => {
                let mut event = Self::basic(
                    "reliability_limit",
                    &e.context.thread_id,
                    &e.context.turn_id,
                    e.timestamp,
                );
                event.is_error = true;
                Some(event)
            }
            RoderEvent::TurnCompleted(e) => Some(Self::basic(
                "turn_completed",
                &e.thread_id,
                &e.turn_id,
                e.timestamp,
            )),
            RoderEvent::TurnFailed(e) => {
                let mut event = Self::basic("turn_failed", &e.thread_id, &e.turn_id, e.timestamp);
                event.is_error = true;
                Some(event)
            }
            _ => None,
        }
    }

    fn basic(
        event_type: impl Into<String>,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        timestamp: OffsetDateTime,
    ) -> Self {
        Self {
            timestamp,
            event_type: event_type.into(),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: None,
            tool_name: None,
            token_usage: None,
            runtime_profile: None,
            speed_policy_phase: None,
            speed_policy_reasoning: None,
            is_error: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{
        InferenceEventReceived, RoderEvent, ToolCallCompleted, ToolCallRequested, TurnStarted,
    };
    use roder_api::inference::{InferenceEvent, RuntimeProfile, TokenUsage};

    use super::*;

    #[test]
    fn trajectory_preserves_turn_tool_and_token_usage_ids() {
        let events = vec![
            RoderEvent::TurnStarted(TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                runtime_profile: RuntimeProfile::Eval,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::ToolCallRequested(ToolCallRequested {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                tool_id: "tool-1".to_string(),
                tool_name: "exec_command".to_string(),
                display_payload: None,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::ToolCallCompleted(ToolCallCompleted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                tool_id: "tool-1".to_string(),
                tool_name: Some("exec_command".to_string()),
                display_payload: None,
                is_error: true,
                output: Some("missing cmd".to_string()),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                event: InferenceEvent::Usage(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    cached_prompt_tokens: 9,
                    cache_creation_prompt_tokens: 1,
                    cache_hit_rate: Some(0.9),
                }),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ];

        let trajectory = EvalTrajectory::from_events("thread-1", "turn-1", &events);

        assert_eq!(trajectory.events.len(), 4);
        assert_eq!(
            trajectory.events[0].runtime_profile.as_deref(),
            Some("eval")
        );
        assert_eq!(trajectory.events[1].tool_id.as_deref(), Some("tool-1"));
        assert!(trajectory.events[2].is_error);
        assert_eq!(
            trajectory.events[3]
                .token_usage
                .as_ref()
                .unwrap()
                .total_tokens,
            15
        );
        let json = serde_json::to_value(&trajectory).unwrap();
        assert_eq!(json["events"][1]["toolName"], "exec_command");
    }

    #[test]
    fn eval_reports_round_trip_failure_classes() {
        let report = EvalReport {
            run: EvalRun {
                suite_id: "tool-schema".to_string(),
                run_id: "run-1".to_string(),
                provider: "mock".to_string(),
                model: "mock".to_string(),
                started_at: OffsetDateTime::UNIX_EPOCH,
                tags: vec!["offline".to_string()],
            },
            outcome: EvalOutcome::Fail,
            failure_class: Some(EvalFailureClass::ToolSchema),
            trajectory: EvalTrajectory {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                events: Vec::new(),
            },
            metrics: vec![EvalMetric {
                name: "tool_errors".to_string(),
                kind: EvalMetricKind::Count,
                value: 1.0,
                unit: None,
            }],
        };

        let json = serde_json::to_string(&report).unwrap();
        let round_trip: EvalReport = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip.outcome, EvalOutcome::Fail);
        assert_eq!(round_trip.failure_class, Some(EvalFailureClass::ToolSchema));
        assert_eq!(round_trip.metrics[0].name, "tool_errors");
    }

    #[test]
    fn eval_report_serde_fixtures_cover_core_outcomes() {
        let cases = [
            (EvalOutcome::Pass, None),
            (EvalOutcome::Fail, Some(EvalFailureClass::ToolSchema)),
            (EvalOutcome::Timeout, Some(EvalFailureClass::Runtime)),
            (
                EvalOutcome::VerifierUncertain,
                Some(EvalFailureClass::Verifier),
            ),
        ];

        for (index, (outcome, failure_class)) in cases.into_iter().enumerate() {
            let report = EvalReport {
                run: EvalRun {
                    suite_id: "phase44-fixtures".to_string(),
                    run_id: format!("run-{index}"),
                    provider: "mock".to_string(),
                    model: "mock".to_string(),
                    started_at: OffsetDateTime::UNIX_EPOCH,
                    tags: vec!["offline".to_string()],
                },
                outcome,
                failure_class,
                trajectory: EvalTrajectory {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    events: Vec::new(),
                },
                metrics: vec![EvalMetric {
                    name: "wall_time_ms".to_string(),
                    kind: EvalMetricKind::Duration,
                    value: 12.0,
                    unit: Some("ms".to_string()),
                }],
            };

            let value = serde_json::to_value(&report).unwrap();
            let round_trip: EvalReport = serde_json::from_value(value).unwrap();

            assert_eq!(round_trip.outcome, report.outcome);
            assert_eq!(round_trip.failure_class, report.failure_class);
        }
    }
}
