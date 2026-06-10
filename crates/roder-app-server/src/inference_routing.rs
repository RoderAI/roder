use std::collections::BTreeMap;

use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TurnId};
use roder_api::inference_routing::{
    InferenceRoutingCostDelta, InferenceRoutingCostEstimate, InferenceRoutingOptionDescriptor,
    InferenceRoutingOutcome,
};
use roder_protocol::{
    InferenceRoutingCostSummary, InferenceRoutingMetricsParams, InferenceRoutingMetricsResult,
    InferenceRoutingRegretSummary, InferenceRoutingStatusParams, InferenceRoutingStatusResult,
    JsonRpcError, RetrievalDebugSummary,
};

use crate::server::AppServer;
use roder_core::inference_routing::{
    RuntimeInferenceRouterConfig, collect_inference_routing_candidates,
    inference_routing_selection_unavailable_reason,
};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

impl AppServer {
    pub(crate) async fn selectable_routing_options(
        &self,
        config: &RuntimeInferenceRouterConfig,
    ) -> Vec<InferenceRoutingOptionDescriptor> {
        if !config.is_active() {
            return Vec::new();
        }
        let candidates = collect_inference_routing_candidates(self.runtime.registry()).await;
        let mut options = self
            .runtime
            .registry()
            .inference_routers
            .iter()
            .flat_map(|router| router.routing_options())
            .filter(|option| {
                config
                    .router_id
                    .as_deref()
                    .is_some_and(|router_id| option.router_id == router_id)
            })
            .filter(|option| option.available)
            .filter(|option| {
                inference_routing_selection_unavailable_reason(&candidates, &option.baseline, false)
                    .is_none()
            })
            .collect::<Vec<_>>();
        options.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
        options
    }

    pub(crate) async fn handle_inference_routing_metrics(
        &self,
        params: InferenceRoutingMetricsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params(format!("unknown thread {:?}", params.thread_id)))?;
        Ok(serde_json::to_value(metrics_result(
            params.thread_id,
            params.turn_id,
            &snapshot.events,
            params.limit,
        ))
        .unwrap())
    }

    pub(crate) async fn handle_inference_routing_status(
        &self,
        params: InferenceRoutingStatusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_thread(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params(format!("unknown thread {:?}", params.thread_id)))?;
        Ok(serde_json::to_value(status_result(
            params.thread_id,
            params.turn_id,
            &snapshot.events,
        ))
        .unwrap())
    }
}

fn status_result(
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    events: &[EventEnvelope],
) -> InferenceRoutingStatusResult {
    let decisions = scoped_events(events, turn_id.as_ref())
        .into_iter()
        .filter_map(|envelope| match &envelope.event {
            RoderEvent::InferenceRoutingDecision(event) => Some(event.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let latest_decision = decisions.last().cloned();
    let decision_count = decisions.len() as u64;
    let active = latest_decision.is_some();
    let router_id = latest_decision
        .as_ref()
        .map(|event| event.decision.router_id.clone());
    let latest_outcome = latest_decision.as_ref().map(|event| event.decision.outcome);
    let default_selection = latest_decision
        .as_ref()
        .map(|event| event.default_selection.clone());
    let selected_selection = latest_decision
        .as_ref()
        .map(|event| event.selected_selection.clone());
    let summary = status_summary(decision_count, latest_decision.as_ref(), turn_id.as_ref());

    InferenceRoutingStatusResult {
        thread_id,
        turn_id,
        active,
        decision_count,
        router_id,
        latest_outcome,
        default_selection,
        selected_selection,
        latest_decision: latest_decision.map(Into::into),
        summary,
    }
}

fn metrics_result(
    thread_id: ThreadId,
    turn_id: TurnId,
    events: &[EventEnvelope],
    limit: Option<usize>,
) -> InferenceRoutingMetricsResult {
    let events = turn_events(events, &turn_id);
    let mut decisions = Vec::new();
    let mut retry_count = 0;
    let mut failure_count = 0;
    let mut turn_failed = false;
    for envelope in events {
        match &envelope.event {
            RoderEvent::InferenceRoutingDecision(event) => decisions.push(event.clone()),
            RoderEvent::ReliabilityRetryRecorded(_) => retry_count += 1,
            RoderEvent::ReliabilityFailureRecorded(_) => failure_count += 1,
            RoderEvent::TurnFailed(_) => turn_failed = true,
            _ => {}
        }
    }

    let decision_count = decisions.len() as u64;
    let mut outcome_counts = BTreeMap::new();
    for event in &decisions {
        *outcome_counts
            .entry(format!("{:?}", event.decision.outcome).to_ascii_lowercase())
            .or_insert(0) += 1;
    }
    let cost_deltas = decisions
        .iter()
        .filter_map(|event| event.decision.cost_delta.clone())
        .collect::<Vec<_>>();
    let cost = cost_summary(&cost_deltas);
    let regret = InferenceRoutingRegretSummary {
        retry_count,
        failure_count,
        turn_failed,
        escalation_count: outcome_count(&outcome_counts, InferenceRoutingOutcome::Escalated),
        fallback_count: outcome_count(&outcome_counts, InferenceRoutingOutcome::Fallback),
    };
    let (decisions, truncated) = bounded(decisions, limit);
    let summary = metrics_summary(decision_count, &cost, &regret, truncated);

    InferenceRoutingMetricsResult {
        thread_id,
        turn_id,
        decisions: decisions.into_iter().map(Into::into).collect(),
        decision_count,
        outcome_counts,
        cost,
        regret,
        cost_deltas,
        summary,
    }
}

fn turn_events<'a>(events: &'a [EventEnvelope], turn_id: &TurnId) -> Vec<&'a EventEnvelope> {
    scoped_events(events, Some(turn_id))
}

fn scoped_events<'a>(
    events: &'a [EventEnvelope],
    turn_id: Option<&TurnId>,
) -> Vec<&'a EventEnvelope> {
    events
        .iter()
        .filter(|event| {
            turn_id.is_none_or(|turn_id| event.turn_id.as_deref() == Some(turn_id.as_str()))
        })
        .collect()
}

fn bounded<T>(mut items: Vec<T>, limit: Option<usize>) -> (Vec<T>, bool) {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let truncated = items.len() > limit;
    items.truncate(limit);
    (items, truncated)
}

fn outcome_count(counts: &BTreeMap<String, u64>, outcome: InferenceRoutingOutcome) -> u64 {
    counts
        .get(&format!("{outcome:?}").to_ascii_lowercase())
        .copied()
        .unwrap_or(0)
}

fn cost_summary(deltas: &[InferenceRoutingCostDelta]) -> InferenceRoutingCostSummary {
    let mut selected_estimated_cost_usd = 0.0;
    let mut baseline_estimated_cost_usd = 0.0;
    let mut estimated_savings_usd = 0.0;
    let mut classifier_overhead_usd = 0.0;
    let mut saw_classifier_overhead = false;
    let mut incomplete_estimate_count = 0;

    for delta in deltas {
        selected_estimated_cost_usd += delta.selected_estimate.total_cost_usd;
        baseline_estimated_cost_usd += delta.baseline_estimate.total_cost_usd;
        estimated_savings_usd += delta.estimated_savings_usd;
        if estimate_incomplete(&delta.selected_estimate)
            || estimate_incomplete(&delta.baseline_estimate)
        {
            incomplete_estimate_count += 1;
        }
        if let Some(overhead) = delta.classifier_overhead_usd {
            saw_classifier_overhead = true;
            classifier_overhead_usd += overhead;
        }
    }

    InferenceRoutingCostSummary {
        selected_estimated_cost_usd,
        baseline_estimated_cost_usd,
        estimated_savings_usd,
        classifier_overhead_usd: saw_classifier_overhead.then_some(classifier_overhead_usd),
        incomplete_estimate_count,
        priced_decision_count: deltas.len() as u64,
    }
}

fn estimate_incomplete(estimate: &InferenceRoutingCostEstimate) -> bool {
    estimate.incomplete
}

fn metrics_summary(
    decision_count: u64,
    cost: &InferenceRoutingCostSummary,
    regret: &InferenceRoutingRegretSummary,
    truncated: bool,
) -> RetrievalDebugSummary {
    let mut notes = Vec::new();
    if decision_count == 0 {
        notes.push("No inference routing decisions were recorded for this turn.".to_string());
    }
    if cost.priced_decision_count == 0 && decision_count > 0 {
        notes.push("No routing decisions carried configured price estimates.".to_string());
    }
    if cost.incomplete_estimate_count > 0 {
        notes.push(format!(
            "{} cost estimate(s) are incomplete because they use estimated input tokens only.",
            cost.incomplete_estimate_count
        ));
    }
    if regret.retry_count > 0 || regret.failure_count > 0 || regret.turn_failed {
        notes.push(format!(
            "Regret signals: {} retry event(s), {} failure event(s), turn_failed={}.",
            regret.retry_count, regret.failure_count, regret.turn_failed
        ));
    }
    if regret.fallback_count > 0 {
        notes.push(format!(
            "{} routing fallback decision(s) kept the default model.",
            regret.fallback_count
        ));
    }

    RetrievalDebugSummary {
        text: format!(
            "{decision_count} routing decision(s), {} priced, estimated savings ${:.6}{}.",
            cost.priced_decision_count,
            cost.estimated_savings_usd,
            if truncated { " in this page" } else { "" }
        ),
        notes,
        truncated,
    }
}

fn status_summary(
    decision_count: u64,
    latest_decision: Option<&roder_api::events::InferenceRoutingDecisionEvent>,
    turn_id: Option<&TurnId>,
) -> RetrievalDebugSummary {
    let mut notes = Vec::new();
    if decision_count == 0 {
        notes.push(match turn_id {
            Some(turn_id) => {
                format!("No inference routing decisions were recorded for turn {turn_id:?}.")
            }
            None => "No inference routing decisions were recorded for this thread.".to_string(),
        });
    }
    let text = match latest_decision {
        Some(event) => format!(
            "{decision_count} routing decision(s); latest {} via router {} selected {}/{}.",
            format!("{:?}", event.decision.outcome).to_ascii_lowercase(),
            event.decision.router_id,
            event.selected_selection.provider,
            event.selected_selection.model
        ),
        None => format!("{decision_count} routing decision(s)."),
    };

    RetrievalDebugSummary {
        text,
        notes,
        truncated: false,
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: err.to_string(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32603,
        message: err.to_string(),
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{EventSource, InferenceRoutingDecisionEvent};
    use roder_api::inference::ModelSelection;
    use roder_api::inference_routing::{
        InferenceRoutingCostDelta, InferenceRoutingCostEstimate, InferenceRoutingDecision,
    };
    use roder_api::reliability::{
        ReliabilityContext, ReliabilityDetails, ReliabilityErrorClass, ReliabilityFailureRecorded,
    };
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn metrics_result_summarizes_cost_and_regret_signals() {
        let thread_id = "thread".to_string();
        let turn_id = "turn".to_string();
        let timestamp = OffsetDateTime::now_utc();
        let selected = ModelSelection {
            provider: "mock".to_string(),
            model: "fast".to_string(),
        };
        let baseline = ModelSelection {
            provider: "mock".to_string(),
            model: "strong".to_string(),
        };
        let mut decision = InferenceRoutingDecision::selected("local", selected.clone(), "routine");
        decision.baseline = Some(baseline.clone());
        decision.cost_delta = Some(InferenceRoutingCostDelta {
            selected_estimate: estimate(selected.clone(), 0.0001),
            baseline_estimate: estimate(baseline, 0.001),
            estimated_savings_usd: 0.0009,
            classifier_overhead_usd: None,
        });
        let events = vec![
            envelope(RoderEvent::InferenceRoutingDecision(
                InferenceRoutingDecisionEvent {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    round_index: 0,
                    default_selection: ModelSelection {
                        provider: "mock".to_string(),
                        model: "strong".to_string(),
                    },
                    selected_selection: selected.clone(),
                    decision,
                    timestamp,
                },
            )),
            envelope(RoderEvent::ReliabilityFailureRecorded(
                ReliabilityFailureRecorded {
                    context: ReliabilityContext {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        ..ReliabilityContext::default()
                    },
                    error_class: ReliabilityErrorClass::ProviderError,
                    details: ReliabilityDetails {
                        message: "provider failed".to_string(),
                        redacted: false,
                    },
                    timestamp,
                },
            )),
        ];

        let result = metrics_result(thread_id, turn_id, &events, Some(10));

        assert_eq!(result.decision_count, 1);
        assert_eq!(result.outcome_counts["selected"], 1);
        assert_eq!(result.cost.priced_decision_count, 1);
        assert!((result.cost.estimated_savings_usd - 0.0009).abs() < f64::EPSILON);
        assert_eq!(result.regret.failure_count, 1);
    }

    #[test]
    fn status_result_reports_latest_routing_decision() {
        let thread_id = "thread".to_string();
        let turn_id = "turn".to_string();
        let selected = ModelSelection {
            provider: "mock".to_string(),
            model: "fast".to_string(),
        };
        let decision = InferenceRoutingDecision::selected("local", selected.clone(), "routine");
        let events = vec![envelope(RoderEvent::InferenceRoutingDecision(
            InferenceRoutingDecisionEvent {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                round_index: 0,
                default_selection: ModelSelection {
                    provider: "mock".to_string(),
                    model: "strong".to_string(),
                },
                selected_selection: selected.clone(),
                decision,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))];

        let result = status_result(thread_id, Some(turn_id), &events);

        assert!(result.active);
        assert_eq!(result.decision_count, 1);
        assert_eq!(result.router_id.as_deref(), Some("local"));
        assert_eq!(result.selected_selection, Some(selected));
        assert!(result.latest_decision.is_some());
        let json = serde_json::to_value(&result.latest_decision).unwrap();
        assert_eq!(json["threadId"], "thread");
        assert_eq!(json["turnId"], "turn");
        assert_eq!(json["defaultSelection"]["provider"], "mock");
    }

    fn estimate(selection: ModelSelection, total_cost_usd: f64) -> InferenceRoutingCostEstimate {
        InferenceRoutingCostEstimate {
            selection,
            prompt_cost_usd: total_cost_usd,
            completion_cost_usd: 0.0,
            total_cost_usd,
            price_source: "test".to_string(),
            usage_source: "test".to_string(),
            incomplete: true,
        }
    }

    fn envelope(event: RoderEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: "event".to_string(),
            seq: 0,
            timestamp: OffsetDateTime::now_utc(),
            source: EventSource::Core,
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        }
    }
}
