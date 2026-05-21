use roder_api::events::RoderEvent;
use roder_api::retrieval::{RetrievalMode, RetrievalOutcomeKind};

use crate::{
    EvalFixture, EvalFixtureResult, EvalMetric, EvalMetricKind, EvalOutcome, EvalSuiteReport,
};

pub fn grade_retrieval_router_fixture(
    fixture: &EvalFixture,
    events: &[RoderEvent],
    outcome: &EvalOutcome,
) -> Vec<EvalMetric> {
    if !fixture.tags.iter().any(|tag| tag == "retrieval-router") {
        return Vec::new();
    }

    let mut planned = 0u64;
    let mut accepted = 0u64;
    let mut ignored = 0u64;
    let mut failed = 0u64;
    let mut useful = 0u64;
    let mut irrelevant = 0u64;
    let mut discovery_before_tool_use = 0u64;
    let mut promotion_before_tool_use = 0u64;
    let mut wrong_tool_family = 0u64;
    let mut unknown_tool = 0u64;
    let mut missing_promotion = 0u64;
    let mut latency_ms = 0u64;
    let mut returned_tokens = 0u64;
    let mut first_useful_path = None;
    let mut first_useful_event = 0u64;

    for (index, event) in events.iter().enumerate() {
        match event {
            RoderEvent::RetrievalRoutePlanned(_) => planned += 1,
            RoderEvent::RetrievalRouteAccepted(_) => accepted += 1,
            RoderEvent::RetrievalRouteIgnored(_) => ignored += 1,
            RoderEvent::RetrievalRouteFailed(_) => failed += 1,
            RoderEvent::RetrievalResultUsed(event) => {
                latency_ms += event.outcome.latency_ms;
                returned_tokens += u64::from(event.outcome.estimated_tokens_returned);
                if event.outcome.discovery_before_tool_use {
                    discovery_before_tool_use += 1;
                }
                if event.outcome.promotion_before_tool_use {
                    promotion_before_tool_use += 1;
                }
                match event.outcome.outcome {
                    RetrievalOutcomeKind::Useful => {
                        useful += 1;
                        if first_useful_path.is_none() {
                            first_useful_path = event
                                .outcome
                                .first_useful_path
                                .clone()
                                .or_else(|| Some(event.outcome.mode.clone()));
                            first_useful_event = index as u64 + 1;
                        }
                    }
                    RetrievalOutcomeKind::Irrelevant => irrelevant += 1,
                    RetrievalOutcomeKind::WrongToolFamily => wrong_tool_family += 1,
                    RetrievalOutcomeKind::UnknownTool => unknown_tool += 1,
                    RetrievalOutcomeKind::MissingPromotion => missing_promotion += 1,
                    _ => {}
                }
                wrong_tool_family += event.outcome.wrong_tool_family_attempts;
            }
            _ => {}
        }
    }

    let mut metrics = vec![
        count_metric("retrieval_router_route_planned", planned),
        count_metric("retrieval_router_route_accepted", accepted),
        count_metric("retrieval_router_route_ignored", ignored),
        count_metric("retrieval_router_route_failed", failed),
        count_metric("retrieval_router_useful_results", useful),
        count_metric("retrieval_router_irrelevant_searches", irrelevant),
        count_metric(
            "retrieval_router_discovery_before_tool_use",
            discovery_before_tool_use,
        ),
        count_metric(
            "retrieval_router_promotion_before_tool_use",
            promotion_before_tool_use,
        ),
        count_metric("retrieval_router_wrong_tool_family", wrong_tool_family),
        count_metric("retrieval_router_unknown_tool", unknown_tool),
        count_metric("retrieval_router_missing_promotion", missing_promotion),
        duration_metric("retrieval_router_latency_ms", latency_ms),
        token_metric("retrieval_router_returned_tokens", returned_tokens),
        count_metric("retrieval_router_first_useful_event", first_useful_event),
        outcome_metric(
            "retrieval_router_final_correctness",
            outcome == &EvalOutcome::Pass,
        ),
        outcome_metric(
            "retrieval_router_on_variant",
            fixture.tags.iter().any(|tag| tag == "router:on"),
        ),
        outcome_metric(
            "retrieval_router_off_variant",
            fixture.tags.iter().any(|tag| tag == "router:off"),
        ),
        outcome_metric(
            "retrieval_router_deferred_context_variant",
            fixture.tags.iter().any(|tag| tag == "context:deferred"),
        ),
        outcome_metric(
            "retrieval_router_full_static_context_variant",
            fixture.tags.iter().any(|tag| tag == "context:full-static"),
        ),
    ];
    for mode in [
        RetrievalMode::ExactText,
        RetrievalMode::FileName,
        RetrievalMode::SemanticCode,
        RetrievalMode::Artifact,
        RetrievalMode::History,
        RetrievalMode::Discovery,
        RetrievalMode::Promotion,
        RetrievalMode::Web,
    ] {
        metrics.push(outcome_metric(
            &format!("retrieval_router_first_useful_path_{}", mode_label(&mode)),
            first_useful_path.as_ref() == Some(&mode),
        ));
    }
    metrics
}

pub(crate) fn retrieval_router_markdown(report: &EvalSuiteReport) -> String {
    let rows = report
        .results
        .iter()
        .filter(|result| {
            result
                .report
                .run
                .tags
                .iter()
                .any(|tag| tag == "retrieval-router")
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }

    let mut text = String::from(
        "\n## Retrieval Router Metrics\n\n| Fixture | Router | Context | Bucket | Planned | Accepted | Ignored | Failed | Useful | Irrelevant | Discovery first | Promotion first | Wrong family | Latency ms | Tokens | Correct |\n| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for result in &rows {
        text.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | `{}` |\n",
            result.fixture_id,
            retrieval_tag(&result.report.run.tags, "router:", "unknown"),
            retrieval_tag(&result.report.run.tags, "context:", "unknown"),
            retrieval_tag(&result.report.run.tags, "bucket:", "none"),
            metric_value(result, "retrieval_router_route_planned"),
            metric_value(result, "retrieval_router_route_accepted"),
            metric_value(result, "retrieval_router_route_ignored"),
            metric_value(result, "retrieval_router_route_failed"),
            metric_value(result, "retrieval_router_useful_results"),
            metric_value(result, "retrieval_router_irrelevant_searches"),
            metric_value(result, "retrieval_router_discovery_before_tool_use"),
            metric_value(result, "retrieval_router_promotion_before_tool_use"),
            metric_value(result, "retrieval_router_wrong_tool_family"),
            metric_value(result, "retrieval_router_latency_ms"),
            metric_value(result, "retrieval_router_returned_tokens"),
            if metric_value(result, "retrieval_router_final_correctness") >= 1.0 {
                "pass"
            } else {
                "fail"
            }
        ));
    }

    text.push_str(
        "\n## Retrieval Router Comparisons\n\n| Slice | Runs | Pass rate | Avg useful results | Avg ignored routes | Avg latency ms | Avg returned tokens |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for (label, prefix, value) in [
        ("router:on", "router:", "on"),
        ("router:off", "router:", "off"),
        ("context:deferred", "context:", "deferred"),
        ("context:full-static", "context:", "full-static"),
        ("bucket:20-50", "bucket:", "20-50"),
        ("bucket:50-100", "bucket:", "50-100"),
        ("bucket:100-plus", "bucket:", "100-plus"),
    ] {
        let slice = rows
            .iter()
            .copied()
            .filter(|result| retrieval_tag(&result.report.run.tags, prefix, "") == value)
            .collect::<Vec<_>>();
        if slice.is_empty() {
            continue;
        }
        text.push_str(&format!(
            "| `{label}` | {} | {:.1}% | {:.1} | {:.1} | {:.1} | {:.1} |\n",
            slice.len(),
            average_metric(&slice, "retrieval_router_final_correctness") * 100.0,
            average_metric(&slice, "retrieval_router_useful_results"),
            average_metric(&slice, "retrieval_router_route_ignored"),
            average_metric(&slice, "retrieval_router_latency_ms"),
            average_metric(&slice, "retrieval_router_returned_tokens"),
        ));
    }
    text.push_str(
        "\nFailure diagnosis: routing failures usually surface as ignored or failed routes; stale or missing indexes surface through retrieval outcome metrics; discovery failures surface as missing promotion, unknown tool, or wrong tool-family noise.\n",
    );
    text
}

fn count_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Count,
        value: value as f64,
        unit: None,
    }
}

fn duration_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Duration,
        value: value as f64,
        unit: Some("ms".to_string()),
    }
}

fn token_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Tokens,
        value: value as f64,
        unit: Some("tokens".to_string()),
    }
}

fn outcome_metric(name: &str, passed: bool) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Outcome,
        value: if passed { 1.0 } else { 0.0 },
        unit: None,
    }
}

fn mode_label(mode: &RetrievalMode) -> &'static str {
    match mode {
        RetrievalMode::ExactText => "exact_text",
        RetrievalMode::FileName => "file_name",
        RetrievalMode::SemanticCode => "semantic_code",
        RetrievalMode::Artifact => "artifact",
        RetrievalMode::History => "history",
        RetrievalMode::Discovery => "discovery",
        RetrievalMode::Promotion => "promotion",
        RetrievalMode::Web => "web",
    }
}

fn retrieval_tag<'a>(tags: &'a [String], prefix: &str, default: &'a str) -> &'a str {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .unwrap_or(default)
}

fn metric_value(result: &EvalFixtureResult, name: &str) -> f64 {
    result
        .report
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.value)
        .unwrap_or(0.0)
}

fn average_metric(rows: &[&EvalFixtureResult], metric: &str) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    rows.iter()
        .map(|result| metric_value(result, metric))
        .sum::<f64>()
        / rows.len() as f64
}

#[cfg(test)]
mod tests {
    use roder_api::events::RoderEvent;
    use roder_api::retrieval::{
        RetrievalIntent, RetrievalMeasuredOutcome, RetrievalOutcomeKind, RetrievalResultUsed,
        RetrievalRoutePlan, RetrievalRoutePlanned,
    };

    use super::*;

    #[test]
    fn retrieval_router_grader_counts_routes_and_noise() {
        let fixture = EvalFixture {
            id: "router".to_string(),
            title: "Router".to_string(),
            prompt: "Find a tool".to_string(),
            tags: vec![
                "retrieval-router".to_string(),
                "router:on".to_string(),
                "context:deferred".to_string(),
            ],
            workspace: Default::default(),
            timeout_ms: None,
            expected: Default::default(),
            constraints: Vec::new(),
            lazy_discovery: None,
        };
        let timestamp = time::OffsetDateTime::UNIX_EPOCH;
        let events = vec![
            RoderEvent::RetrievalRoutePlanned(RetrievalRoutePlanned {
                plan: RetrievalRoutePlan {
                    route_id: "route".to_string(),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                    intent: RetrievalIntent::InspectTool,
                    recommended: Vec::new(),
                    avoid: Vec::new(),
                    timestamp,
                },
            }),
            RoderEvent::RetrievalResultUsed(RetrievalResultUsed {
                outcome: RetrievalMeasuredOutcome {
                    route_id: "route".to_string(),
                    mode: RetrievalMode::Discovery,
                    tool: "discovery.search".to_string(),
                    outcome: RetrievalOutcomeKind::Useful,
                    first_useful_path: Some(RetrievalMode::Discovery),
                    discovery_before_tool_use: true,
                    promotion_before_tool_use: false,
                    wrong_tool_family_attempts: 1,
                    result_count: 2,
                    latency_ms: 12,
                    bytes_returned: 100,
                    estimated_tokens_returned: 25,
                },
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                timestamp,
            }),
        ];

        let metrics = grade_retrieval_router_fixture(&fixture, &events, &EvalOutcome::Pass);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap_or_default()
        };
        assert_eq!(value("retrieval_router_route_planned"), 1.0);
        assert_eq!(value("retrieval_router_useful_results"), 1.0);
        assert_eq!(value("retrieval_router_discovery_before_tool_use"), 1.0);
        assert_eq!(value("retrieval_router_wrong_tool_family"), 1.0);
        assert_eq!(value("retrieval_router_first_useful_path_discovery"), 1.0);
    }
}
