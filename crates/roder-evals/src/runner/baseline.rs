use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::report::{EvalFixtureResult, EvalSuiteReport, list_eval_reports};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityBaseline {
    pub version: u32,
    #[serde(default)]
    pub unknown_error_blocker_threshold: u64,
    #[serde(default)]
    pub expectations: Vec<ReliabilityBaselineExpectation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityBaselineExpectation {
    pub scope: String,
    pub metric: String,
    #[serde(default)]
    pub max_count: u64,
    #[serde(default)]
    pub max_increase: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityBaselineComparison {
    pub status: ReliabilityBaselineStatus,
    pub rows: Vec<ReliabilityBaselineRow>,
    pub unknown_errors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityBaselineStatus {
    Ok,
    Attention,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityBaselineRow {
    pub scope: String,
    pub metric: String,
    pub current: u64,
    pub allowed: u64,
    pub status: ReliabilityBaselineStatus,
}

pub fn compare_eval_report_to_baseline(
    output_dir: &Path,
    report_id: &str,
    baseline_path: &Path,
) -> anyhow::Result<String> {
    let report = read_suite_report(output_dir, report_id)?;
    let baseline: ReliabilityBaseline =
        serde_json::from_str(&std::fs::read_to_string(baseline_path)?)?;
    let comparison = compare_reliability_baseline(&report, &baseline);
    Ok(reliability_baseline_markdown(&comparison))
}

pub fn compare_reliability_baseline(
    report: &EvalSuiteReport,
    baseline: &ReliabilityBaseline,
) -> ReliabilityBaselineComparison {
    let scoped = scoped_reliability_metrics(report);
    let unknown_errors = metric_value(&scoped, "suite", "reliability_unknown_errors");
    let mut rows = Vec::new();
    let mut status = if unknown_errors > baseline.unknown_error_blocker_threshold {
        ReliabilityBaselineStatus::Blocked
    } else {
        ReliabilityBaselineStatus::Ok
    };
    for expectation in &baseline.expectations {
        let current = metric_value(&scoped, &expectation.scope, &expectation.metric);
        let allowed = expectation
            .max_count
            .saturating_add(expectation.max_increase);
        let row_status = if expectation.metric == "reliability_unknown_errors"
            && current > expectation.max_count
        {
            ReliabilityBaselineStatus::Blocked
        } else if current > allowed {
            ReliabilityBaselineStatus::Attention
        } else {
            ReliabilityBaselineStatus::Ok
        };
        status = merge_status(status, &row_status);
        rows.push(ReliabilityBaselineRow {
            scope: expectation.scope.clone(),
            metric: expectation.metric.clone(),
            current,
            allowed,
            status: row_status,
        });
    }
    ReliabilityBaselineComparison {
        status,
        rows,
        unknown_errors,
    }
}

fn read_suite_report(output_dir: &Path, report_id: &str) -> anyhow::Result<EvalSuiteReport> {
    let reports = list_eval_reports(output_dir)?;
    let summary = reports
        .into_iter()
        .find(|report| report.id == report_id)
        .ok_or_else(|| anyhow::anyhow!("eval report not found: {report_id}"))?;
    let path = summary.path.join("eval-run.json");
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn reliability_baseline_markdown(comparison: &ReliabilityBaselineComparison) -> String {
    let mut text = format!(
        "\n## Reliability Baseline Comparison\n\nStatus: `{:?}`\n\nUnknown errors: `{}`\n\n| Scope | Metric | Current | Allowed | Status |\n| --- | --- | ---: | ---: | --- |\n",
        comparison.status, comparison.unknown_errors
    );
    for row in &comparison.rows {
        text.push_str(&format!(
            "| `{}` | `{}` | {} | {} | `{:?}` |\n",
            row.scope, row.metric, row.current, row.allowed, row.status
        ));
    }
    text
}

fn scoped_reliability_metrics(report: &EvalSuiteReport) -> BTreeMap<(String, String), u64> {
    let mut scoped = BTreeMap::new();
    for result in &report.results {
        add_result_metrics(&mut scoped, "suite", result);
        let model = format!(
            "model:{}/{}",
            result.report.run.provider, result.report.run.model
        );
        add_result_metrics(&mut scoped, &model, result);
        for tag in result
            .report
            .run
            .tags
            .iter()
            .filter(|tag| tag.starts_with("tool:"))
        {
            add_result_metrics(&mut scoped, tag, result);
        }
    }
    scoped
}

fn add_result_metrics(
    scoped: &mut BTreeMap<(String, String), u64>,
    scope: &str,
    result: &EvalFixtureResult,
) {
    for metric in &result.report.metrics {
        if !metric.name.starts_with("reliability_") {
            continue;
        }
        *scoped
            .entry((scope.to_string(), metric.name.clone()))
            .or_insert(0) += metric.value.max(0.0) as u64;
    }
}

fn metric_value(scoped: &BTreeMap<(String, String), u64>, scope: &str, metric: &str) -> u64 {
    scoped
        .get(&(scope.to_string(), metric.to_string()))
        .copied()
        .unwrap_or(0)
}

fn merge_status(
    current: ReliabilityBaselineStatus,
    next: &ReliabilityBaselineStatus,
) -> ReliabilityBaselineStatus {
    match (current, next) {
        (ReliabilityBaselineStatus::Blocked, _) | (_, ReliabilityBaselineStatus::Blocked) => {
            ReliabilityBaselineStatus::Blocked
        }
        (ReliabilityBaselineStatus::Attention, _) | (_, ReliabilityBaselineStatus::Attention) => {
            ReliabilityBaselineStatus::Attention
        }
        _ => ReliabilityBaselineStatus::Ok,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roder_api::events::{RoderEvent, ThreadId, TurnId};
    use roder_api::reliability::{
        ReliabilityContext, ReliabilityDetails, ReliabilityErrorClass, ReliabilityFailureRecorded,
    };
    use time::OffsetDateTime;

    use crate::{EvalMetric, EvalMetricKind, EvalOutcome, EvalReport, EvalRun, EvalTrajectory};

    use super::*;

    #[test]
    fn reliability_baseline_flags_unknown_errors_as_blockers() {
        let report = suite_report(vec![fixture_result(
            "unknown-panic",
            vec![
                metric("reliability_unknown_errors", 1.0),
                metric("reliability_error_class_unknown", 1.0),
            ],
            EvalOutcome::HarnessError,
        )]);
        let baseline = ReliabilityBaseline {
            version: 1,
            unknown_error_blocker_threshold: 0,
            expectations: vec![ReliabilityBaselineExpectation {
                scope: "suite".to_string(),
                metric: "reliability_unknown_errors".to_string(),
                max_count: 0,
                max_increase: 0,
            }],
        };

        let comparison = compare_reliability_baseline(&report, &baseline);

        assert_eq!(comparison.status, ReliabilityBaselineStatus::Blocked);
        assert_eq!(comparison.unknown_errors, 1);
        assert_eq!(
            comparison.rows[0].status,
            ReliabilityBaselineStatus::Blocked
        );
    }

    #[test]
    fn reliability_baseline_flags_per_model_spikes() {
        let report = suite_report(vec![fixture_result(
            "provider-429",
            vec![metric("reliability_error_class_provider_error", 3.0)],
            EvalOutcome::Pass,
        )]);
        let baseline = ReliabilityBaseline {
            version: 1,
            unknown_error_blocker_threshold: 0,
            expectations: vec![ReliabilityBaselineExpectation {
                scope: "model:mock/mock".to_string(),
                metric: "reliability_error_class_provider_error".to_string(),
                max_count: 1,
                max_increase: 1,
            }],
        };

        let comparison = compare_reliability_baseline(&report, &baseline);

        assert_eq!(comparison.status, ReliabilityBaselineStatus::Attention);
        assert_eq!(comparison.rows[0].current, 3);
        assert_eq!(comparison.rows[0].allowed, 2);
    }

    fn suite_report(results: Vec<EvalFixtureResult>) -> EvalSuiteReport {
        EvalSuiteReport {
            suite_id: "reliability".to_string(),
            fixture_dir: PathBuf::from("evals/fixtures/reliability"),
            output_dir: PathBuf::from("/tmp/roder-evals"),
            offline: true,
            generated_at: OffsetDateTime::UNIX_EPOCH,
            results,
        }
    }

    fn fixture_result(
        fixture_id: &str,
        metrics: Vec<EvalMetric>,
        outcome: EvalOutcome,
    ) -> EvalFixtureResult {
        EvalFixtureResult {
            fixture_id: fixture_id.to_string(),
            title: fixture_id.to_string(),
            workspace: PathBuf::from("/tmp/workspace"),
            final_answer: String::new(),
            report: EvalReport {
                run: EvalRun {
                    suite_id: "reliability".to_string(),
                    run_id: "run".to_string(),
                    provider: "mock".to_string(),
                    model: "mock".to_string(),
                    started_at: OffsetDateTime::UNIX_EPOCH,
                    tags: vec!["reliability".to_string(), "tool:read_file".to_string()],
                },
                outcome,
                failure_class: None,
                trajectory: EvalTrajectory::from_events(
                    ThreadId::from("thread"),
                    TurnId::from("turn"),
                    &[RoderEvent::ReliabilityFailureRecorded(
                        ReliabilityFailureRecorded {
                            context: ReliabilityContext {
                                thread_id: "thread".to_string(),
                                turn_id: "turn".to_string(),
                                tool_id: None,
                                tool_name: None,
                                provider: Some("mock".to_string()),
                                model: Some("mock".to_string()),
                            },
                            error_class: ReliabilityErrorClass::Unknown,
                            details: ReliabilityDetails::redacted("test"),
                            timestamp: OffsetDateTime::UNIX_EPOCH,
                        },
                    )],
                ),
                metrics,
            },
            trace_excerpt: Vec::new(),
            failure_message: None,
        }
    }

    fn metric(name: &str, value: f64) -> EvalMetric {
        EvalMetric {
            name: name.to_string(),
            kind: EvalMetricKind::Count,
            value,
            unit: None,
        }
    }
}
