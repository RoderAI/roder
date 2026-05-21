use std::collections::BTreeMap;

use roder_api::events::{RoderEvent, ThreadId, TurnFailed, TurnId};
use roder_api::inference::InferenceEvent;
use roder_api::reliability::{
    ReliabilityContext, ReliabilityDetails, ReliabilityErrorClass, ReliabilityFailureRecorded,
    ReliabilityLimitDecision, ReliabilityLimitKind, ReliabilityLimitRecorded,
    ReliabilityRetryDecision, ReliabilityRetryRecorded,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{EvalFailureClass, EvalFixture, EvalMetric, EvalMetricKind, EvalOutcome};

use super::report::{EvalFixtureResult, EvalSuiteReport};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityReportSummary {
    #[serde(default)]
    pub error_class_counts: BTreeMap<String, u64>,
    pub retry_attempts: u64,
    pub retry_recoveries: u64,
    pub failure_limit_stops: u64,
    pub unknown_errors: u64,
}

pub(super) struct FixtureReliabilityInjection {
    pub events: Vec<RoderEvent>,
    pub outcome: Option<EvalOutcome>,
    pub failure_class: Option<EvalFailureClass>,
    pub failure_message: Option<String>,
}

pub(super) fn fixture_reliability_injection(
    fixture: &EvalFixture,
    thread_id: &ThreadId,
    turn_id: &TurnId,
) -> Option<FixtureReliabilityInjection> {
    let tag = fixture
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("reliability:"))?;
    let context = context(thread_id, turn_id);
    let timestamp = OffsetDateTime::now_utc();
    match tag {
        "invalid_arguments" => Some(FixtureReliabilityInjection {
            events: vec![RoderEvent::ReliabilityFailureRecorded(
                ReliabilityFailureRecorded {
                    context,
                    error_class: ReliabilityErrorClass::InvalidArguments,
                    details: ReliabilityDetails::redacted("missing required tool field path"),
                    timestamp,
                },
            )],
            outcome: Some(EvalOutcome::Fail),
            failure_class: Some(EvalFailureClass::ToolSchema),
            failure_message: Some("invalid tool arguments were classified".to_string()),
        }),
        "missing_file" => Some(FixtureReliabilityInjection {
            events: vec![RoderEvent::ReliabilityFailureRecorded(
                ReliabilityFailureRecorded {
                    context,
                    error_class: ReliabilityErrorClass::UnexpectedEnvironment,
                    details: ReliabilityDetails::redacted("missing file src/missing.rs"),
                    timestamp,
                },
            )],
            outcome: Some(EvalOutcome::Fail),
            failure_class: Some(EvalFailureClass::Environment),
            failure_message: Some("missing file was classified as environment failure".to_string()),
        }),
        "provider_empty_body" => Some(FixtureReliabilityInjection {
            events: vec![retry_event(
                context,
                1,
                2,
                Some(0),
                "empty provider body",
                timestamp,
            )],
            outcome: None,
            failure_class: None,
            failure_message: None,
        }),
        "provider_429" => Some(FixtureReliabilityInjection {
            events: vec![retry_event(context, 1, 3, Some(0), "status_429", timestamp)],
            outcome: None,
            failure_class: None,
            failure_message: None,
        }),
        "repeated_timeout" => Some(FixtureReliabilityInjection {
            events: vec![
                RoderEvent::ReliabilityLimitRecorded(ReliabilityLimitRecorded {
                    context: context.clone(),
                    error_class: ReliabilityErrorClass::Timeout,
                    limit_kind: ReliabilityLimitKind::ModelCallsPerTurn,
                    decision: ReliabilityLimitDecision::StopTurn,
                    current: 3,
                    limit: 3,
                    details: ReliabilityDetails::redacted("repeated timeout limit reached"),
                    timestamp,
                }),
                RoderEvent::TurnFailed(TurnFailed {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    error: "repeated timeout limit reached".to_string(),
                    error_kind: Some("reliability_limit".to_string()),
                    timestamp,
                }),
            ],
            outcome: Some(EvalOutcome::Timeout),
            failure_class: Some(EvalFailureClass::Runtime),
            failure_message: Some("repeated timeout limit reached".to_string()),
        }),
        "unknown_panic" => Some(FixtureReliabilityInjection {
            events: vec![RoderEvent::ReliabilityFailureRecorded(
                ReliabilityFailureRecorded {
                    context,
                    error_class: ReliabilityErrorClass::Unknown,
                    details: ReliabilityDetails::redacted("panic converted to unknown failure"),
                    timestamp,
                },
            )],
            outcome: Some(EvalOutcome::HarnessError),
            failure_class: Some(EvalFailureClass::Unknown),
            failure_message: Some("unknown panic conversion was classified".to_string()),
        }),
        _ => None,
    }
}

pub(super) fn reliability_metrics(events: &[RoderEvent], outcome: &EvalOutcome) -> Vec<EvalMetric> {
    let summary = summarize_events(events, outcome);
    let mut metrics = vec![
        count_metric("reliability_retry_attempts", summary.retry_attempts),
        count_metric("reliability_retry_recoveries", summary.retry_recoveries),
        count_metric(
            "reliability_failure_limit_stops",
            summary.failure_limit_stops,
        ),
        count_metric("reliability_unknown_errors", summary.unknown_errors),
    ];
    for (class, count) in summary.error_class_counts {
        metrics.push(count_metric(
            &format!("reliability_error_class_{class}"),
            count,
        ));
    }
    metrics
}

pub(super) fn reliability_summary(report: &EvalSuiteReport) -> ReliabilityReportSummary {
    report.results.iter().fold(
        ReliabilityReportSummary::default(),
        |mut summary, result| {
            let current = summarize_result(result);
            merge_summary(&mut summary, current);
            summary
        },
    )
}

pub(super) fn reliability_markdown(report: &EvalSuiteReport) -> String {
    let summary = reliability_summary(report);
    let mut text = String::from("\n## Reliability Metrics\n\n| Metric | Value |\n| --- | ---: |\n");
    text.push_str(&format!(
        "| Retry attempts | {} |\n| Retry recoveries | {} |\n| Failure-limit stops | {} |\n| Unknown errors | {} |\n",
        summary.retry_attempts,
        summary.retry_recoveries,
        summary.failure_limit_stops,
        summary.unknown_errors
    ));
    text.push_str("\n| Error class | Count |\n| --- | ---: |\n");
    for (class, count) in &summary.error_class_counts {
        text.push_str(&format!("| `{class}` | {count} |\n"));
    }
    text.push_str(
        "\n| Fixture | Outcome | Retry attempts | Limit stops | Unknown errors |\n| --- | --- | ---: | ---: | ---: |\n",
    );
    for result in &report.results {
        let current = summarize_result(result);
        text.push_str(&format!(
            "| `{}` | `{:?}` | {} | {} | {} |\n",
            result.fixture_id,
            result.report.outcome,
            current.retry_attempts,
            current.failure_limit_stops,
            current.unknown_errors
        ));
    }
    text
}

fn summarize_result(result: &EvalFixtureResult) -> ReliabilityReportSummary {
    let mut summary = ReliabilityReportSummary::default();
    for metric in &result.report.metrics {
        let value = metric.value.max(0.0) as u64;
        match metric.name.as_str() {
            "reliability_retry_attempts" => summary.retry_attempts = value,
            "reliability_retry_recoveries" => summary.retry_recoveries = value,
            "reliability_failure_limit_stops" => summary.failure_limit_stops = value,
            "reliability_unknown_errors" => summary.unknown_errors = value,
            name => {
                if let Some(class) = name.strip_prefix("reliability_error_class_") {
                    summary.error_class_counts.insert(class.to_string(), value);
                }
            }
        }
    }
    summary
}

fn summarize_events(events: &[RoderEvent], outcome: &EvalOutcome) -> ReliabilityReportSummary {
    let mut summary = ReliabilityReportSummary::default();
    for event in events {
        match event {
            RoderEvent::ReliabilityRetryRecorded(retry) => {
                summary.retry_attempts += 1;
                add_class(&mut summary, retry.error_class);
            }
            RoderEvent::ReliabilityFailureRecorded(failure) => {
                add_class(&mut summary, failure.error_class);
                if failure.error_class == ReliabilityErrorClass::Unknown {
                    summary.unknown_errors += 1;
                }
            }
            RoderEvent::ReliabilityLimitRecorded(limit) => {
                add_class(&mut summary, limit.error_class);
                if limit.decision != ReliabilityLimitDecision::Continue {
                    summary.failure_limit_stops += 1;
                }
                if limit.error_class == ReliabilityErrorClass::Unknown {
                    summary.unknown_errors += 1;
                }
            }
            RoderEvent::InferenceEventReceived(received) => {
                if provider_metadata_is_retry(&received.event) {
                    summary.retry_attempts += 1;
                    *summary
                        .error_class_counts
                        .entry(error_class_key(ReliabilityErrorClass::ProviderError))
                        .or_insert(0) += 1;
                }
            }
            _ => {}
        }
    }
    if *outcome == EvalOutcome::Pass && summary.retry_attempts > 0 {
        summary.retry_recoveries = 1;
    }
    summary
}

fn provider_metadata_is_retry(event: &InferenceEvent) -> bool {
    matches!(
        event,
        InferenceEvent::ProviderMetadata(metadata)
            if metadata.get("kind").and_then(serde_json::Value::as_str)
                == Some("reliability_retry_attempt")
    )
}

fn merge_summary(target: &mut ReliabilityReportSummary, source: ReliabilityReportSummary) {
    target.retry_attempts += source.retry_attempts;
    target.retry_recoveries += source.retry_recoveries;
    target.failure_limit_stops += source.failure_limit_stops;
    target.unknown_errors += source.unknown_errors;
    for (class, count) in source.error_class_counts {
        *target.error_class_counts.entry(class).or_insert(0) += count;
    }
}

fn retry_event(
    context: ReliabilityContext,
    attempt: u32,
    max_attempts: u32,
    delay_ms: Option<u64>,
    details: &str,
    timestamp: OffsetDateTime,
) -> RoderEvent {
    RoderEvent::ReliabilityRetryRecorded(ReliabilityRetryRecorded {
        context,
        error_class: ReliabilityErrorClass::ProviderError,
        decision: ReliabilityRetryDecision::Retry,
        attempt,
        max_attempts,
        delay_ms,
        details: ReliabilityDetails::redacted(details),
        timestamp,
    })
}

fn context(thread_id: &ThreadId, turn_id: &TurnId) -> ReliabilityContext {
    ReliabilityContext {
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        tool_id: None,
        tool_name: None,
        provider: Some("mock".to_string()),
        model: Some("mock".to_string()),
    }
}

fn add_class(summary: &mut ReliabilityReportSummary, class: ReliabilityErrorClass) {
    *summary
        .error_class_counts
        .entry(error_class_key(class))
        .or_insert(0) += 1;
}

fn error_class_key(class: ReliabilityErrorClass) -> String {
    serde_json::to_value(class)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{class:?}"))
}

fn count_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Count,
        value: value as f64,
        unit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EvalReport, EvalRun, EvalTrajectory};

    #[test]
    fn reliability_fixture_injection_classifies_unknown_panics() {
        let fixture = EvalFixture {
            id: "unknown".to_string(),
            title: "Unknown".to_string(),
            prompt: "Classify unknown panic".to_string(),
            tags: vec!["reliability:unknown_panic".to_string()],
            workspace: Default::default(),
            timeout_ms: None,
            expected: Default::default(),
            constraints: Vec::new(),
            lazy_discovery: None,
        };

        let injection =
            fixture_reliability_injection(&fixture, &"thread".to_string(), &"turn".to_string())
                .unwrap();

        assert_eq!(injection.outcome, Some(EvalOutcome::HarnessError));
        assert_eq!(injection.failure_class, Some(EvalFailureClass::Unknown));
        assert!(matches!(
            injection.events[0],
            RoderEvent::ReliabilityFailureRecorded(_)
        ));
    }

    #[test]
    fn reliability_summary_counts_retries_limits_and_unknowns() {
        let thread_id = "thread".to_string();
        let turn_id = "turn".to_string();
        let events = vec![
            retry_event(
                context(&thread_id, &turn_id),
                1,
                3,
                Some(0),
                "status_429",
                OffsetDateTime::UNIX_EPOCH,
            ),
            RoderEvent::ReliabilityLimitRecorded(ReliabilityLimitRecorded {
                context: context(&thread_id, &turn_id),
                error_class: ReliabilityErrorClass::Unknown,
                limit_kind: ReliabilityLimitKind::ModelCallsPerTurn,
                decision: ReliabilityLimitDecision::StopTurn,
                current: 1,
                limit: 1,
                details: ReliabilityDetails::redacted("unknown"),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ];
        let trajectory = EvalTrajectory::from_events(&thread_id, &turn_id, &events);
        let result = EvalFixtureResult {
            fixture_id: "provider-429".to_string(),
            title: "Provider 429".to_string(),
            workspace: std::path::PathBuf::from("/tmp/workspace"),
            final_answer: String::new(),
            report: EvalReport {
                run: EvalRun {
                    suite_id: "reliability".to_string(),
                    run_id: "run".to_string(),
                    provider: "mock".to_string(),
                    model: "mock".to_string(),
                    started_at: OffsetDateTime::UNIX_EPOCH,
                    tags: vec!["reliability".to_string()],
                },
                outcome: EvalOutcome::Pass,
                failure_class: None,
                trajectory,
                metrics: reliability_metrics(&events, &EvalOutcome::Pass),
            },
            trace_excerpt: Vec::new(),
            failure_message: None,
        };
        let report = EvalSuiteReport {
            suite_id: "reliability".to_string(),
            fixture_dir: std::path::PathBuf::from("evals/fixtures/reliability"),
            output_dir: std::path::PathBuf::from("/tmp/reports"),
            offline: true,
            generated_at: OffsetDateTime::UNIX_EPOCH,
            results: vec![result],
        };

        let summary = reliability_summary(&report);

        assert_eq!(summary.retry_attempts, 1);
        assert_eq!(summary.retry_recoveries, 1);
        assert_eq!(summary.failure_limit_stops, 1);
        assert_eq!(summary.unknown_errors, 1);
        assert_eq!(summary.error_class_counts["provider_error"], 1);
        assert_eq!(summary.error_class_counts["unknown"], 1);
    }
}
