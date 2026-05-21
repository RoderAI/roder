use roder_api::events::{ThreadId, TurnId};
use roder_api::retrieval::{
    RetrievalMeasuredOutcome, RetrievalMode, RetrievalOutcomeKind, RetrievalResultUsed,
    RetrievalRouteAccepted, RetrievalRouteFailed, RetrievalRouteId, RetrievalRouteIgnored,
};
use serde_json::Value;
use time::OffsetDateTime;

pub(crate) fn retrieval_route_id(thread_id: &ThreadId, turn_id: &TurnId) -> RetrievalRouteId {
    format!("route:{thread_id}:{turn_id}")
}

pub(crate) fn retrieval_mode_for_tool(tool_name: &str) -> Option<RetrievalMode> {
    match tool_name {
        "grep" => Some(RetrievalMode::ExactText),
        "glob" => Some(RetrievalMode::FileName),
        "read_artifact" | "grep_artifact" | "tail_artifact" => Some(RetrievalMode::Artifact),
        "discovery.list" | "discovery.search" => Some(RetrievalMode::Discovery),
        "discovery.read" => Some(RetrievalMode::Promotion),
        "history.search" | "memory.query" | "memory.read" => Some(RetrievalMode::History),
        "code_index.search" | "semantic_code.search" => Some(RetrievalMode::SemanticCode),
        "web_search" => Some(RetrievalMode::Web),
        _ => None,
    }
}

pub(crate) fn route_choice_event(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_name: &str,
    arguments: &Value,
) -> roder_api::events::RoderEvent {
    let route_id = retrieval_route_id(thread_id, turn_id);
    if let Some(mode) = retrieval_mode_for_tool(tool_name) {
        roder_api::events::RoderEvent::RetrievalRouteAccepted(RetrievalRouteAccepted {
            route_id,
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            mode,
            tool: tool_name.to_string(),
            query: query_from_arguments(arguments),
            timestamp: OffsetDateTime::now_utc(),
        })
    } else {
        roder_api::events::RoderEvent::RetrievalRouteIgnored(RetrievalRouteIgnored {
            route_id,
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            chosen_tool: tool_name.to_string(),
            recommended_modes: Vec::new(),
            reason: "tool has no retrieval mode metadata".to_string(),
            timestamp: OffsetDateTime::now_utc(),
        })
    }
}

pub(crate) fn route_failed_event(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_name: &str,
    reason: impl Into<String>,
) -> roder_api::events::RoderEvent {
    roder_api::events::RoderEvent::RetrievalRouteFailed(RetrievalRouteFailed {
        route_id: retrieval_route_id(thread_id, turn_id),
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        mode: retrieval_mode_for_tool(tool_name).unwrap_or(RetrievalMode::Discovery),
        tool: tool_name.to_string(),
        reason: reason.into(),
        timestamp: OffsetDateTime::now_utc(),
    })
}

pub(crate) fn result_used_event(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_name: &str,
    data: &Value,
    output: &str,
    is_error: bool,
) -> Option<roder_api::events::RoderEvent> {
    let mode = retrieval_mode_for_tool(tool_name)?;
    Some(roder_api::events::RoderEvent::RetrievalResultUsed(
        RetrievalResultUsed {
            outcome: RetrievalMeasuredOutcome {
                route_id: retrieval_route_id(thread_id, turn_id),
                mode: mode.clone(),
                tool: tool_name.to_string(),
                outcome: outcome_kind(&mode, data, output, is_error),
                first_useful_path: (!is_error).then_some(mode.clone()),
                discovery_before_tool_use: matches!(
                    mode,
                    RetrievalMode::Discovery | RetrievalMode::Promotion
                ),
                promotion_before_tool_use: matches!(mode, RetrievalMode::Promotion),
                wrong_tool_family_attempts: 0,
                result_count: result_count(data),
                latency_ms: u64_value(data, "elapsed_ms"),
                bytes_returned: output.len() as u64,
                estimated_tokens_returned: estimate_tokens(output),
            },
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        },
    ))
}

pub(crate) fn unknown_tool_result_event(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_name: &str,
) -> roder_api::events::RoderEvent {
    roder_api::events::RoderEvent::RetrievalResultUsed(RetrievalResultUsed {
        outcome: RetrievalMeasuredOutcome {
            route_id: retrieval_route_id(thread_id, turn_id),
            mode: RetrievalMode::Discovery,
            tool: tool_name.to_string(),
            outcome: RetrievalOutcomeKind::UnknownTool,
            first_useful_path: None,
            discovery_before_tool_use: false,
            promotion_before_tool_use: false,
            wrong_tool_family_attempts: 1,
            result_count: 0,
            latency_ms: 0,
            bytes_returned: 0,
            estimated_tokens_returned: 0,
        },
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        timestamp: OffsetDateTime::now_utc(),
    })
}

fn outcome_kind(
    mode: &RetrievalMode,
    data: &Value,
    output: &str,
    is_error: bool,
) -> RetrievalOutcomeKind {
    if is_error {
        if output.contains("auth") || output.contains("unauthorized") {
            return RetrievalOutcomeKind::AuthRequired;
        }
        return RetrievalOutcomeKind::Failed;
    }
    if data.get("stale").and_then(Value::as_bool) == Some(true) {
        return RetrievalOutcomeKind::StaleIndex;
    }
    if matches!(mode, RetrievalMode::Promotion)
        && data.get("promoted").and_then(Value::as_bool) == Some(false)
    {
        return RetrievalOutcomeKind::MissingPromotion;
    }
    if result_count(data) == 0
        && matches!(
            mode,
            RetrievalMode::ExactText
                | RetrievalMode::FileName
                | RetrievalMode::SemanticCode
                | RetrievalMode::Discovery
        )
    {
        return RetrievalOutcomeKind::Irrelevant;
    }
    RetrievalOutcomeKind::Useful
}

fn result_count(data: &Value) -> u64 {
    for key in [
        "result_count",
        "match_count",
        "total_lines",
        "total",
        "groupCount",
    ] {
        if let Some(value) = u64_value_opt(data, key) {
            return value;
        }
    }
    if let Some(matches) = data.get("matches").and_then(Value::as_array) {
        return matches.len() as u64;
    }
    if let Some(items) = data.get("items").and_then(Value::as_array) {
        return items.len() as u64;
    }
    0
}

fn query_from_arguments(arguments: &Value) -> String {
    for key in ["query", "pattern", "item_id", "artifact_id"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            return value.to_string();
        }
    }
    String::new()
}

fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

fn u64_value(data: &Value, key: &str) -> u64 {
    u64_value_opt(data, key).unwrap_or_default()
}

fn u64_value_opt(data: &Value, key: &str) -> Option<u64> {
    data.get(key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_metrics_accept_recommended_search_path() {
        let event = route_choice_event(
            &"thread".to_string(),
            &"turn".to_string(),
            "grep",
            &serde_json::json!({ "query": "Needle" }),
        );

        let roder_api::events::RoderEvent::RetrievalRouteAccepted(event) = event else {
            panic!("expected accepted route");
        };
        assert_eq!(event.mode, RetrievalMode::ExactText);
        assert_eq!(event.query, "Needle");
    }

    #[test]
    fn retrieval_metrics_record_unrecommended_paths_and_unknown_tools() {
        let ignored = route_choice_event(
            &"thread".to_string(),
            &"turn".to_string(),
            "write_file",
            &serde_json::json!({}),
        );
        assert!(matches!(
            ignored,
            roder_api::events::RoderEvent::RetrievalRouteIgnored(_)
        ));

        let unknown =
            unknown_tool_result_event(&"thread".to_string(), &"turn".to_string(), "mcp.bad");
        let roder_api::events::RoderEvent::RetrievalResultUsed(event) = unknown else {
            panic!("expected retrieval result");
        };
        assert_eq!(event.outcome.outcome, RetrievalOutcomeKind::UnknownTool);
        assert_eq!(event.outcome.wrong_tool_family_attempts, 1);
    }

    #[test]
    fn retrieval_metrics_record_result_counts_latency_and_tokens() {
        let event = result_used_event(
            &"thread".to_string(),
            &"turn".to_string(),
            "grep",
            &serde_json::json!({
                "matches": ["src/lib.rs:1:Needle"],
                "elapsed_ms": 7
            }),
            "src/lib.rs:1:Needle",
            false,
        )
        .unwrap();

        let roder_api::events::RoderEvent::RetrievalResultUsed(event) = event else {
            panic!("expected retrieval result");
        };
        assert_eq!(event.outcome.outcome, RetrievalOutcomeKind::Useful);
        assert_eq!(event.outcome.result_count, 1);
        assert_eq!(event.outcome.latency_ms, 7);
        assert!(event.outcome.estimated_tokens_returned > 0);
    }
}
