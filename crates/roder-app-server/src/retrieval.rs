use std::collections::BTreeMap;

use roder_api::discovery::DiscoveryPromotionRecord;
use roder_api::events::{EventEnvelope, RoderEvent, TurnId};
use roder_api::retrieval::{RetrievalMeasuredOutcome, RetrievalOutcomeKind, RetrievalRoutePlan};
use roder_protocol::{
    JsonRpcError, RetrievalDebugSummary, RetrievalMetricsResult, RetrievalPromotedCapabilityState,
    RetrievalPromotedResult, RetrievalRecommendationsResult, RetrievalTurnParams,
};

use crate::server::AppServer;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

impl AppServer {
    pub(crate) async fn handle_retrieval_recommendations(
        &self,
        params: RetrievalTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params(format!("unknown thread {:?}", params.thread_id)))?;
        let events = turn_events(&snapshot.events, &params.turn_id);
        let mut plans = Vec::new();
        for envelope in events {
            if let RoderEvent::RetrievalRoutePlanned(event) = &envelope.event {
                plans.push(event.plan.clone());
            }
        }
        let (plans, truncated) = bounded(plans, params.limit);
        let summary = recommendations_summary(&plans, truncated);
        Ok(serde_json::to_value(RetrievalRecommendationsResult {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            plans,
            summary,
        })
        .unwrap())
    }

    pub(crate) async fn handle_retrieval_metrics(
        &self,
        params: RetrievalTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params(format!("unknown thread {:?}", params.thread_id)))?;
        let events = turn_events(&snapshot.events, &params.turn_id);
        let mut outcomes = Vec::new();
        let mut accepted_count = 0;
        let mut ignored_count = 0;
        let mut failed_count = 0;
        for envelope in events {
            match &envelope.event {
                RoderEvent::RetrievalRouteAccepted(_) => accepted_count += 1,
                RoderEvent::RetrievalRouteIgnored(_) => ignored_count += 1,
                RoderEvent::RetrievalRouteFailed(_) => failed_count += 1,
                RoderEvent::RetrievalResultUsed(event) => outcomes.push(event.outcome.clone()),
                _ => {}
            }
        }
        let (outcomes, truncated) = bounded(outcomes, params.limit);
        let mut outcome_counts = BTreeMap::new();
        let mut mode_counts = BTreeMap::new();
        for outcome in &outcomes {
            *outcome_counts
                .entry(format!("{:?}", outcome.outcome).to_ascii_lowercase())
                .or_insert(0) += 1;
            *mode_counts.entry(outcome.mode.clone()).or_insert(0) += 1;
        }
        let summary = metrics_summary(
            &outcomes,
            accepted_count,
            ignored_count,
            failed_count,
            truncated,
        );
        Ok(serde_json::to_value(RetrievalMetricsResult {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            outcomes,
            accepted_count,
            ignored_count,
            failed_count,
            outcome_counts,
            mode_counts,
            summary,
        })
        .unwrap())
    }

    pub(crate) async fn handle_retrieval_promoted(
        &self,
        params: RetrievalTurnParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params(format!("unknown thread {:?}", params.thread_id)))?;
        let events = turn_events(&snapshot.events, &params.turn_id);
        let mut states = Vec::new();
        for envelope in events {
            match &envelope.event {
                RoderEvent::DiscoveryItemPromoted(event) => {
                    states.push(promotion_state(&event.record, None, None))
                }
                RoderEvent::DiscoveryPromotionReused(event) => {
                    states.push(promotion_state(&event.record, None, None))
                }
                RoderEvent::DiscoveryWarmCacheHit(event) => {
                    states.push(promotion_state(&event.record, None, None))
                }
                RoderEvent::DiscoveryPromotionExpired(event) => {
                    states.push(promotion_state(&event.record, None, None))
                }
                RoderEvent::RetrievalDiscoveryItemPromoted(event) => {
                    states.push(RetrievalPromotedCapabilityState {
                        item_id: event.item_id.clone(),
                        route_id: Some(event.route_id.clone()),
                        state: "promoted".to_string(),
                        cache_status: None,
                        reason: None,
                        thread_id: event.thread_id.clone(),
                        turn_id: Some(event.turn_id.clone()),
                        timestamp: event.timestamp,
                    });
                }
                RoderEvent::RetrievalPromotionSkipped(event) => {
                    states.push(RetrievalPromotedCapabilityState {
                        item_id: event.item_id.clone(),
                        route_id: Some(event.route_id.clone()),
                        state: "skipped".to_string(),
                        cache_status: None,
                        reason: Some(event.reason.clone()),
                        thread_id: event.thread_id.clone(),
                        turn_id: Some(event.turn_id.clone()),
                        timestamp: event.timestamp,
                    });
                }
                _ => {}
            }
        }
        let (states, truncated) = bounded(states, params.limit);
        let summary = promoted_summary(&states, truncated);
        Ok(serde_json::to_value(RetrievalPromotedResult {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            states,
            summary,
        })
        .unwrap())
    }
}

fn turn_events<'a>(events: &'a [EventEnvelope], turn_id: &TurnId) -> Vec<&'a EventEnvelope> {
    events
        .iter()
        .filter(|event| event.turn_id.as_deref() == Some(turn_id.as_str()))
        .collect()
}

fn bounded<T>(mut items: Vec<T>, limit: Option<usize>) -> (Vec<T>, bool) {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let truncated = items.len() > limit;
    items.truncate(limit);
    (items, truncated)
}

fn recommendations_summary(plans: &[RetrievalRoutePlan], truncated: bool) -> RetrievalDebugSummary {
    let mut notes = Vec::new();
    if plans.is_empty() {
        notes.push("No retrieval route recommendations were recorded for this turn.".to_string());
    }
    for plan in plans.iter().take(5) {
        if plan.recommended.is_empty() {
            notes.push(format!(
                "{} has no recommended retrieval tools.",
                plan.route_id
            ));
        } else {
            let tools = plan
                .recommended
                .iter()
                .map(|rec| rec.tool.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            notes.push(format!("{} recommends {tools}.", plan.route_id));
        }
    }
    RetrievalDebugSummary {
        text: format!(
            "{} route recommendation(s) recorded{}.",
            plans.len(),
            if truncated { " in this page" } else { "" }
        ),
        notes,
        truncated,
    }
}

fn metrics_summary(
    outcomes: &[RetrievalMeasuredOutcome],
    accepted_count: u64,
    ignored_count: u64,
    failed_count: u64,
    truncated: bool,
) -> RetrievalDebugSummary {
    let mut notes = Vec::new();
    if ignored_count > 0 {
        notes.push(format!(
            "{ignored_count} model tool choice(s) ignored the recommended retrieval route."
        ));
    }
    if failed_count > 0 {
        notes.push(format!(
            "{failed_count} recommended retrieval route(s) failed before producing context."
        ));
    }
    for outcome in outcomes {
        match outcome.outcome {
            RetrievalOutcomeKind::StaleIndex => notes.push(format!(
                "{} returned stale index evidence via {}.",
                outcome.route_id, outcome.tool
            )),
            RetrievalOutcomeKind::MissingIndex => notes.push(format!(
                "{} could not use indexed search because the index was missing.",
                outcome.route_id
            )),
            RetrievalOutcomeKind::MissingPromotion => notes.push(format!(
                "{} used {} before promotion.",
                outcome.route_id, outcome.tool
            )),
            RetrievalOutcomeKind::WrongToolFamily => notes.push(format!(
                "{} used a wrong retrieval tool family: {}.",
                outcome.route_id, outcome.tool
            )),
            _ => {}
        }
    }
    RetrievalDebugSummary {
        text: format!(
            "{accepted_count} accepted, {ignored_count} ignored, {failed_count} failed, {} measured outcome(s){}.",
            outcomes.len(),
            if truncated { " in this page" } else { "" }
        ),
        notes,
        truncated,
    }
}

fn promotion_state(
    record: &DiscoveryPromotionRecord,
    route_id: Option<String>,
    reason: Option<String>,
) -> RetrievalPromotedCapabilityState {
    RetrievalPromotedCapabilityState {
        item_id: record.item_id.clone(),
        route_id,
        state: format!("{:?}", record.promotion).to_ascii_lowercase(),
        cache_status: Some(format!("{:?}", record.cache_status).to_ascii_lowercase()),
        reason,
        thread_id: record.thread_id.clone(),
        turn_id: record.turn_id.clone(),
        timestamp: record.timestamp,
    }
}

fn promoted_summary(
    states: &[RetrievalPromotedCapabilityState],
    truncated: bool,
) -> RetrievalDebugSummary {
    let mut notes = Vec::new();
    if states.is_empty() {
        notes.push("No discovery promotion or skip state was recorded for this turn.".to_string());
    }
    for state in states.iter().take(5) {
        match state.state.as_str() {
            "skipped" => notes.push(format!(
                "{} promotion skipped: {}.",
                state.item_id,
                state.reason.as_deref().unwrap_or("no reason recorded")
            )),
            "warmcachehit" | "warm_cache_hit" => {
                notes.push(format!("{} was already warm-cached.", state.item_id));
            }
            _ => notes.push(format!(
                "{} promotion state: {}.",
                state.item_id, state.state
            )),
        }
    }
    RetrievalDebugSummary {
        text: format!(
            "{} promoted capability state(s) recorded{}.",
            states.len(),
            if truncated { " in this page" } else { "" }
        ),
        notes,
        truncated,
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = err.to_string();
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{EventEnvelope, EventSource};
    use roder_api::retrieval::{
        RetrievalConfidence, RetrievalIntent, RetrievalRecommendation, RetrievalRoutePlanned,
    };
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn retrieval_summaries_are_bounded_and_explain_misses() {
        let outcome = RetrievalMeasuredOutcome {
            route_id: "route-1".to_string(),
            mode: roder_api::retrieval::RetrievalMode::SemanticCode,
            tool: "code_index.search".to_string(),
            outcome: RetrievalOutcomeKind::StaleIndex,
            first_useful_path: None,
            discovery_before_tool_use: false,
            promotion_before_tool_use: false,
            wrong_tool_family_attempts: 0,
            result_count: 0,
            latency_ms: 10,
            bytes_returned: 0,
            estimated_tokens_returned: 0,
        };
        let summary = metrics_summary(&[outcome], 0, 1, 0, false);
        assert!(summary.text.contains("ignored"));
        assert!(
            summary
                .notes
                .iter()
                .any(|note| note.contains("stale index"))
        );

        let (items, truncated) = bounded(vec![1, 2, 3], Some(2));
        assert_eq!(items, vec![1, 2]);
        assert!(truncated);
    }

    #[test]
    fn retrieval_turn_event_filter_reads_route_plans() {
        let plan = RetrievalRoutePlan {
            route_id: "route-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            intent: RetrievalIntent::InspectTool,
            recommended: vec![RetrievalRecommendation {
                mode: roder_api::retrieval::RetrievalMode::Discovery,
                tool: "discovery.search".to_string(),
                query: "grep".to_string(),
                reason: "tool lookup".to_string(),
                confidence: RetrievalConfidence::High,
                item_id: None,
            }],
            avoid: Vec::new(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };
        let envelope = EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: "retrieval/routePlanned".to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            event: RoderEvent::RetrievalRoutePlanned(RetrievalRoutePlanned { plan }),
        };
        assert_eq!(turn_events(&[envelope], &"turn-1".to_string()).len(), 1);
    }
}
