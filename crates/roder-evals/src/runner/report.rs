use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roder_api::events::RoderEvent;
use roder_api::inference::InferenceEvent;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::retrieval_router::retrieval_router_markdown;
use crate::{EvalMetric, EvalMetricKind, EvalOutcome, EvalTrajectory, EvalTrajectoryEvent};

use super::lazy_discovery::lazy_discovery_markdown;
use super::reliability::{
    ReliabilityReportSummary, reliability_markdown, reliability_metrics, reliability_summary,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalSuiteReport {
    pub suite_id: String,
    pub fixture_dir: PathBuf,
    pub output_dir: PathBuf,
    pub offline: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub results: Vec<EvalFixtureResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalFixtureResult {
    pub fixture_id: String,
    pub title: String,
    pub workspace: PathBuf,
    pub final_answer: String,
    pub report: crate::EvalReport,
    #[serde(default)]
    pub trace_excerpt: Vec<EvalTrajectoryEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportSummary {
    pub id: String,
    pub path: PathBuf,
    pub suite_id: String,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    #[serde(default)]
    pub reliability: ReliabilityReportSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportDocument {
    pub summary: EvalReportSummary,
    pub markdown: String,
    pub truncated: bool,
}

pub fn write_eval_report_files(report: &EvalSuiteReport, output_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(
        output_dir.join("eval-run.json"),
        serde_json::to_string_pretty(report)?,
    )?;
    std::fs::write(
        output_dir.join("eval-report.md"),
        eval_report_markdown(report),
    )?;
    Ok(())
}

pub fn list_eval_reports(output_dir: &Path) -> anyhow::Result<Vec<EvalReportSummary>> {
    let mut reports = Vec::new();
    collect_eval_reports(output_dir, output_dir, &mut reports)?;
    reports.sort_by_key(|report| Reverse(report.generated_at));
    Ok(reports)
}

pub fn read_eval_report(
    output_dir: &Path,
    report_id: &str,
    max_bytes: usize,
) -> anyhow::Result<EvalReportDocument> {
    let reports = list_eval_reports(output_dir)?;
    let summary = reports
        .into_iter()
        .find(|report| report.id == report_id)
        .ok_or_else(|| anyhow::anyhow!("eval report not found: {report_id}"))?;
    let markdown_path = summary.path.join("eval-report.md");
    let markdown = std::fs::read_to_string(&markdown_path)?;
    let truncated = markdown.len() > max_bytes;
    let markdown = if truncated {
        markdown.chars().take(max_bytes).collect()
    } else {
        markdown
    };
    Ok(EvalReportDocument {
        summary,
        markdown,
        truncated,
    })
}

pub(super) fn eval_metrics(
    events: &[RoderEvent],
    wall_time_ms: u128,
    outcome: &EvalOutcome,
) -> Vec<EvalMetric> {
    let search = search_metrics(events);
    let model_calls = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::InferenceStarted(_)))
        .count();
    let tool_calls = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::ToolCallRequested(_)))
        .count();
    let tool_errors = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RoderEvent::ToolCallCompleted(completed) if completed.is_error
            )
        })
        .count();
    let child_tasks = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RoderEvent::TaskStarted(_)
                    | RoderEvent::SubagentStarted(_)
                    | RoderEvent::TeamMemberStarted(_)
            )
        })
        .count();
    let deadline_remaining_seconds = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::InferenceStarted(started) => started.deadline_remaining_seconds,
            _ => None,
        })
        .next_back()
        .unwrap_or(0);
    let total_tokens = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::InferenceEventReceived(received) => match &received.event {
                InferenceEvent::Usage(usage) => Some(u64::from(usage.total_tokens)),
                _ => None,
            },
            _ => None,
        })
        .sum::<u64>();
    let context_tokens = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::ContextAssemblyCompleted(completed) => {
                Some(u64::from(completed.estimated_tokens))
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let context_bytes = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::ContextAssemblyCompleted(completed) => Some(completed.total_byte_count),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let entrypoint_candidates = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::ContextEntrypointCandidatesInjected(injected) => {
                Some(injected.candidate_count)
            }
            _ => None,
        })
        .sum::<u64>();
    let entrypoint_injection_event = events
        .iter()
        .position(|event| matches!(event, RoderEvent::ContextEntrypointCandidatesInjected(_)))
        .map(|index| index as u64 + 1)
        .unwrap_or(0);
    let first_relevant_file_read = events
        .iter()
        .position(is_relevant_file_read)
        .map(|index| index as u64 + 1)
        .unwrap_or(0);
    let irrelevant_file_reads = events
        .iter()
        .filter(|event| is_file_read(event) && !is_relevant_file_read(event))
        .count() as u64;
    let truncation_follow_ups = count_truncation_follow_ups(events);
    let tool_output_truncations = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::ToolOutputTruncated(_)))
        .count() as u64;
    let task_ledger_updates = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::TaskLedgerUpdated(_)))
        .count() as u64;
    let task_ledger_tasks = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::TaskLedgerUpdated(updated) => Some(updated.tasks.len() as u64),
            _ => None,
        })
        .next_back()
        .unwrap_or(0);
    let task_ledger_completed = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::TaskLedgerUpdated(updated) => Some(updated.completed_count),
            _ => None,
        })
        .next_back()
        .unwrap_or(0);
    let verification_required = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::VerificationRequired(_)))
        .count() as u64;
    let verification_completed = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RoderEvent::VerificationCompleted(completed) if completed.passed
            )
        })
        .count() as u64;
    let verification_failed = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RoderEvent::VerificationCompleted(completed) if !completed.passed
            )
        })
        .count() as u64;
    let verification_skipped = events
        .iter()
        .filter(|event| matches!(event, RoderEvent::VerificationSkipped(_)))
        .count() as u64;
    let verification_open_gaps = events
        .iter()
        .filter_map(|event| match event {
            RoderEvent::VerificationCompleted(completed) => Some(completed.open_gaps.len() as u64),
            RoderEvent::VerificationRequired(required) => Some(required.open_gaps.len() as u64),
            _ => None,
        })
        .next_back()
        .unwrap_or(0);
    let mut metrics = vec![
        EvalMetric {
            name: "outcome_pass".to_string(),
            kind: EvalMetricKind::Outcome,
            value: if outcome == &EvalOutcome::Pass {
                1.0
            } else {
                0.0
            },
            unit: None,
        },
        EvalMetric {
            name: "wall_time_ms".to_string(),
            kind: EvalMetricKind::Duration,
            value: wall_time_ms as f64,
            unit: Some("ms".to_string()),
        },
        EvalMetric {
            name: "model_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: model_calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "tool_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: tool_calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "child_task_count".to_string(),
            kind: EvalMetricKind::Count,
            value: child_tasks as f64,
            unit: None,
        },
        EvalMetric {
            name: "deadline_remaining_seconds".to_string(),
            kind: EvalMetricKind::Duration,
            value: deadline_remaining_seconds as f64,
            unit: Some("s".to_string()),
        },
        EvalMetric {
            name: "tool_errors".to_string(),
            kind: EvalMetricKind::Count,
            value: tool_errors as f64,
            unit: None,
        },
        EvalMetric {
            name: "total_tokens".to_string(),
            kind: EvalMetricKind::Tokens,
            value: total_tokens as f64,
            unit: Some("tokens".to_string()),
        },
        EvalMetric {
            name: "context_estimated_tokens".to_string(),
            kind: EvalMetricKind::Tokens,
            value: context_tokens as f64,
            unit: Some("tokens".to_string()),
        },
        EvalMetric {
            name: "context_bytes".to_string(),
            kind: EvalMetricKind::Bytes,
            value: context_bytes as f64,
            unit: Some("bytes".to_string()),
        },
        EvalMetric {
            name: "entrypoint_candidates".to_string(),
            kind: EvalMetricKind::Count,
            value: entrypoint_candidates as f64,
            unit: None,
        },
        EvalMetric {
            name: "entrypoint_injection_event".to_string(),
            kind: EvalMetricKind::Count,
            value: entrypoint_injection_event as f64,
            unit: None,
        },
        EvalMetric {
            name: "first_relevant_file_read_event".to_string(),
            kind: EvalMetricKind::Count,
            value: first_relevant_file_read as f64,
            unit: None,
        },
        EvalMetric {
            name: "irrelevant_file_reads".to_string(),
            kind: EvalMetricKind::Count,
            value: irrelevant_file_reads as f64,
            unit: None,
        },
        EvalMetric {
            name: "truncation_follow_ups".to_string(),
            kind: EvalMetricKind::Count,
            value: truncation_follow_ups as f64,
            unit: None,
        },
        EvalMetric {
            name: "tool_output_truncations".to_string(),
            kind: EvalMetricKind::Count,
            value: tool_output_truncations as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: search.calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_indexed_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: search.indexed_calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_scan_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: search.scan_calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_fallback_calls".to_string(),
            kind: EvalMetricKind::Count,
            value: search.fallback_calls as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_candidate_files".to_string(),
            kind: EvalMetricKind::Count,
            value: search.candidate_files as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_verified_files".to_string(),
            kind: EvalMetricKind::Count,
            value: search.verified_files as f64,
            unit: None,
        },
        EvalMetric {
            name: "grep_elapsed_ms".to_string(),
            kind: EvalMetricKind::Duration,
            value: search.elapsed_ms as f64,
            unit: Some("ms".to_string()),
        },
        EvalMetric {
            name: "grep_index_bytes".to_string(),
            kind: EvalMetricKind::Bytes,
            value: search.index_bytes as f64,
            unit: Some("bytes".to_string()),
        },
        EvalMetric {
            name: "grep_index_build_time_ms".to_string(),
            kind: EvalMetricKind::Duration,
            value: search.index_build_time_ms as f64,
            unit: Some("ms".to_string()),
        },
        EvalMetric {
            name: "task_ledger_updates".to_string(),
            kind: EvalMetricKind::Count,
            value: task_ledger_updates as f64,
            unit: None,
        },
        EvalMetric {
            name: "task_ledger_tasks".to_string(),
            kind: EvalMetricKind::Count,
            value: task_ledger_tasks as f64,
            unit: None,
        },
        EvalMetric {
            name: "task_ledger_completed".to_string(),
            kind: EvalMetricKind::Count,
            value: task_ledger_completed as f64,
            unit: None,
        },
        EvalMetric {
            name: "verification_required".to_string(),
            kind: EvalMetricKind::Count,
            value: verification_required as f64,
            unit: None,
        },
        EvalMetric {
            name: "verification_completed".to_string(),
            kind: EvalMetricKind::Count,
            value: verification_completed as f64,
            unit: None,
        },
        EvalMetric {
            name: "verification_failed".to_string(),
            kind: EvalMetricKind::Count,
            value: verification_failed as f64,
            unit: None,
        },
        EvalMetric {
            name: "verification_skipped".to_string(),
            kind: EvalMetricKind::Count,
            value: verification_skipped as f64,
            unit: None,
        },
        EvalMetric {
            name: "verification_open_gaps".to_string(),
            kind: EvalMetricKind::Count,
            value: verification_open_gaps as f64,
            unit: None,
        },
    ];
    metrics.extend(reliability_metrics(events, outcome));
    metrics
}

#[derive(Default)]
struct SearchEvalMetrics {
    calls: u64,
    indexed_calls: u64,
    scan_calls: u64,
    fallback_calls: u64,
    candidate_files: u64,
    verified_files: u64,
    elapsed_ms: u64,
    index_bytes: u64,
    index_build_time_ms: u64,
}

fn search_metrics(events: &[RoderEvent]) -> SearchEvalMetrics {
    let mut metrics = SearchEvalMetrics::default();
    for event in events {
        let RoderEvent::ToolCallCompleted(completed) = event else {
            continue;
        };
        if completed.tool_name.as_deref() != Some("grep") {
            continue;
        }
        metrics.calls += 1;
        let Some(payload) = completed.display_payload.as_ref() else {
            continue;
        };
        match payload.get("engine").and_then(serde_json::Value::as_str) {
            Some("indexed") => metrics.indexed_calls += 1,
            Some("scan") => metrics.scan_calls += 1,
            Some("fallback") => metrics.fallback_calls += 1,
            _ => {}
        }
        metrics.candidate_files += u64_payload(payload, "candidate_files");
        metrics.verified_files += u64_payload(payload, "verified_files");
        metrics.elapsed_ms += u64_payload(payload, "elapsed_ms");
        metrics.index_bytes = metrics.index_bytes.max(u64_payload(payload, "index_bytes"));
        metrics.index_build_time_ms = metrics
            .index_build_time_ms
            .max(u64_payload(payload, "index_build_time_ms"));
    }
    metrics
}

fn u64_payload(payload: &serde_json::Value, key: &str) -> u64 {
    payload
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}

fn is_file_read(event: &RoderEvent) -> bool {
    matches!(
        event,
        RoderEvent::ToolCallCompleted(completed)
            if completed.tool_name.as_deref() == Some("read_file")
    )
}

fn is_relevant_file_read(event: &RoderEvent) -> bool {
    matches!(
        event,
        RoderEvent::ToolCallCompleted(completed)
            if completed.tool_name.as_deref() == Some("read_file")
                && completed
                    .display_payload
                    .as_ref()
                    .is_some_and(|payload| payload.to_string().contains("relevant"))
    )
}

fn count_truncation_follow_ups(events: &[RoderEvent]) -> u64 {
    let mut saw_truncation = false;
    let mut follow_ups = 0u64;
    for event in events {
        match event {
            RoderEvent::ToolOutputTruncated(_) => saw_truncation = true,
            RoderEvent::ToolCallRequested(requested)
                if saw_truncation
                    && matches!(requested.tool_name.as_str(), "read_file" | "grep" | "glob") =>
            {
                follow_ups += 1;
                saw_truncation = false;
            }
            _ => {}
        }
    }
    follow_ups
}

fn collect_eval_reports(
    root: &Path,
    dir: &Path,
    reports: &mut Vec<EvalReportSummary>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let run_path = dir.join("eval-run.json");
    if run_path.exists() {
        let report: EvalSuiteReport = serde_json::from_str(&std::fs::read_to_string(&run_path)?)?;
        let id = if dir == root {
            "eval-run".to_string()
        } else {
            dir.strip_prefix(root)
                .unwrap_or(dir)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        };
        reports.push(summary_from_report(id, dir.to_path_buf(), &report));
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_eval_reports(root, &path, reports)?;
        }
    }
    Ok(())
}

fn summary_from_report(id: String, path: PathBuf, report: &EvalSuiteReport) -> EvalReportSummary {
    let passed = report
        .results
        .iter()
        .filter(|result| result.report.outcome == EvalOutcome::Pass)
        .count();
    EvalReportSummary {
        id,
        path,
        suite_id: report.suite_id.clone(),
        fixture_count: report.results.len(),
        passed,
        failed: report.results.len().saturating_sub(passed),
        reliability: reliability_summary(report),
        generated_at: report.generated_at,
    }
}

pub(super) fn trajectory_excerpt(trajectory: &EvalTrajectory) -> Vec<EvalTrajectoryEvent> {
    let start = trajectory.events.len().saturating_sub(8);
    trajectory.events[start..].to_vec()
}

fn eval_report_markdown(report: &EvalSuiteReport) -> String {
    let passed = report
        .results
        .iter()
        .filter(|result| result.report.outcome == EvalOutcome::Pass)
        .count();
    let mut text = format!(
        "# Roder Eval Report\n\n- Suite: `{}`\n- Fixtures: {}\n- Passed: {}\n- Failed: {}\n",
        report.suite_id,
        report.results.len(),
        passed,
        report.results.len().saturating_sub(passed)
    );
    text.push_str(
        "\n## Pass Rates\n\n| Scope | Passed | Total | Pass rate |\n| --- | ---: | ---: | ---: |\n",
    );
    for (scope, passed, total) in pass_rate_rows(report) {
        let rate = if total == 0 {
            0.0
        } else {
            (passed as f64 / total as f64) * 100.0
        };
        text.push_str(&format!(
            "| `{scope}` | {passed} | {total} | {rate:.1}% |\n"
        ));
    }
    text.push_str(
        "\n## Fixtures\n\n| Fixture | Outcome | Failure class | Trace excerpt |\n| --- | --- | --- | --- |\n",
    );
    for result in &report.results {
        let class = result
            .report
            .failure_class
            .as_ref()
            .map(|class| format!("{class:?}"))
            .unwrap_or_else(|| "-".to_string());
        let excerpt = result
            .trace_excerpt
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        text.push_str(&format!(
            "| `{}` | `{:?}` | `{}` | {} |\n",
            result.fixture_id, result.report.outcome, class, excerpt
        ));
        if let Some(message) = &result.failure_message {
            text.push_str(&format!(
                "\nFailure `{}`: {}\n\n",
                result.fixture_id,
                message.replace('\n', " ")
            ));
        }
    }
    text.push_str(
        "\n## Speed Metrics\n\n| Fixture | Policy | Wall ms | Model calls | Tool calls | Child tasks | Deadline remaining s | Outcome |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for result in &report.results {
        text.push_str(&format!(
            "| `{}` | `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | `{:?}` |\n",
            result.fixture_id,
            speed_policy_label(result),
            metric_value(result, "wall_time_ms"),
            metric_value(result, "model_calls"),
            metric_value(result, "tool_calls"),
            metric_value(result, "child_task_count"),
            metric_value(result, "deadline_remaining_seconds"),
            result.report.outcome,
        ));
    }
    let comparisons = speed_policy_comparisons(report);
    if !comparisons.is_empty() {
        text.push_str(
            "\n## Speed Policy Comparison\n\n| Fixture | Baseline wall ms | Speed wall ms | Delta ms | Baseline model calls | Speed model calls | Baseline tool calls | Speed tool calls | Baseline child tasks | Speed child tasks | Quality |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
        );
        for comparison in comparisons {
            text.push_str(&format!(
                "| `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {} |\n",
                comparison.fixture_id,
                comparison.baseline_wall_ms,
                comparison.speed_wall_ms,
                comparison.speed_wall_ms - comparison.baseline_wall_ms,
                comparison.baseline_model_calls,
                comparison.speed_model_calls,
                comparison.baseline_tool_calls,
                comparison.speed_tool_calls,
                comparison.baseline_child_tasks,
                comparison.speed_child_tasks,
                comparison.quality,
            ));
        }
    }
    text.push_str(
        "\n## Search Metrics\n\n| Fixture | Grep calls | Indexed | Scan | Fallback | Candidate files | Verified files | Grep elapsed ms | Index bytes | Index build ms |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for result in &report.results {
        text.push_str(&format!(
            "| `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |\n",
            result.fixture_id,
            metric_value(result, "grep_calls"),
            metric_value(result, "grep_indexed_calls"),
            metric_value(result, "grep_scan_calls"),
            metric_value(result, "grep_fallback_calls"),
            metric_value(result, "grep_candidate_files"),
            metric_value(result, "grep_verified_files"),
            metric_value(result, "grep_elapsed_ms"),
            metric_value(result, "grep_index_bytes"),
            metric_value(result, "grep_index_build_time_ms"),
        ));
    }
    let profile_comparisons = model_profile_comparisons(report);
    if !profile_comparisons.is_empty() {
        text.push_str(
            "\n## Model Profile Deltas\n\n| Fixture | Profile | Outcome | Failure class | Wall ms | Model calls | Tool calls |\n| --- | --- | --- | --- | ---: | ---: | ---: |\n",
        );
        for comparison in profile_comparisons {
            text.push_str(&format!(
                "| `{}` | `{}` | `{:?}` | `{}` | {:.0} | {:.0} | {:.0} |\n",
                comparison.fixture_id,
                comparison.profile,
                comparison.outcome,
                comparison.failure_class,
                comparison.wall_ms,
                comparison.model_calls,
                comparison.tool_calls,
            ));
        }
        text.push_str("\nRecommended profile changes should be made only when this table shows an improved failure class or equivalent quality with lower wall/model/tool cost.\n");
    }
    text.push_str(
        "\n## Context Metrics\n\n| Fixture | Context tokens | Context bytes | Entrypoint candidates | Entrypoint injection event | First relevant read event | Irrelevant reads | Truncation follow-ups | Tool output truncations |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for result in &report.results {
        text.push_str(&format!(
            "| `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |\n",
            result.fixture_id,
            metric_value(result, "context_estimated_tokens"),
            metric_value(result, "context_bytes"),
            metric_value(result, "entrypoint_candidates"),
            metric_value(result, "entrypoint_injection_event"),
            metric_value(result, "first_relevant_file_read_event"),
            metric_value(result, "irrelevant_file_reads"),
            metric_value(result, "truncation_follow_ups"),
            metric_value(result, "tool_output_truncations"),
        ));
    }
    text.push_str(
        "\n## Task Ledger Metrics\n\n| Fixture | Updates | Tasks | Completed |\n| --- | ---: | ---: | ---: |\n",
    );
    for result in &report.results {
        text.push_str(&format!(
            "| `{}` | {:.0} | {:.0} | {:.0} |\n",
            result.fixture_id,
            metric_value(result, "task_ledger_updates"),
            metric_value(result, "task_ledger_tasks"),
            metric_value(result, "task_ledger_completed"),
        ));
    }
    text.push_str(
        "\n## Verification Metrics\n\n| Fixture | Required | Completed | Failed | Skipped | Open gaps | Remaining gaps |\n| --- | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for result in &report.results {
        text.push_str(&format!(
            "| `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {} |\n",
            result.fixture_id,
            metric_value(result, "verification_required"),
            metric_value(result, "verification_completed"),
            metric_value(result, "verification_failed"),
            metric_value(result, "verification_skipped"),
            metric_value(result, "verification_open_gaps"),
            markdown_cell(&verification_remaining_gaps(
                &result.report.trajectory.events
            )),
        ));
    }
    text.push_str(&retrieval_router_markdown(report));
    text.push_str(&lazy_discovery_markdown(report));
    text.push_str(&reliability_markdown(report));
    let groups = failure_groups(report);
    if !groups.is_empty() {
        text.push_str("\n## Failure Groups\n\n| Tool | Model | Failure class | Count |\n| --- | --- | --- | --- |\n");
        for ((tool, model, class), count) in groups {
            text.push_str(&format!("| `{tool}` | `{model}` | `{class}` | {count} |\n"));
        }
    }
    text
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

fn speed_policy_label(result: &EvalFixtureResult) -> &'static str {
    if result
        .report
        .run
        .tags
        .iter()
        .any(|tag| tag == "speed_policy:on")
    {
        "on"
    } else {
        "off"
    }
}

fn profile_label(result: &EvalFixtureResult) -> Option<String> {
    result
        .report
        .run
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("profile:").map(str::to_string))
}

struct ModelProfileComparison {
    fixture_id: String,
    profile: String,
    outcome: EvalOutcome,
    failure_class: String,
    wall_ms: f64,
    model_calls: f64,
    tool_calls: f64,
}

fn model_profile_comparisons(report: &EvalSuiteReport) -> Vec<ModelProfileComparison> {
    let mut rows = report
        .results
        .iter()
        .filter_map(|result| {
            let profile = profile_label(result)?;
            Some(ModelProfileComparison {
                fixture_id: result.fixture_id.clone(),
                profile,
                outcome: result.report.outcome.clone(),
                failure_class: result
                    .report
                    .failure_class
                    .as_ref()
                    .map(|class| format!("{class:?}"))
                    .unwrap_or_else(|| "-".to_string()),
                wall_ms: metric_value(result, "wall_time_ms"),
                model_calls: metric_value(result, "model_calls"),
                tool_calls: metric_value(result, "tool_calls"),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.fixture_id
            .cmp(&right.fixture_id)
            .then_with(|| left.profile.cmp(&right.profile))
    });
    rows
}

struct SpeedPolicyComparison {
    fixture_id: String,
    baseline_wall_ms: f64,
    speed_wall_ms: f64,
    baseline_model_calls: f64,
    speed_model_calls: f64,
    baseline_tool_calls: f64,
    speed_tool_calls: f64,
    baseline_child_tasks: f64,
    speed_child_tasks: f64,
    quality: String,
}

fn speed_policy_comparisons(report: &EvalSuiteReport) -> Vec<SpeedPolicyComparison> {
    let mut by_fixture =
        BTreeMap::<String, (Option<&EvalFixtureResult>, Option<&EvalFixtureResult>)>::new();
    for result in &report.results {
        let entry = by_fixture
            .entry(result.fixture_id.clone())
            .or_insert((None, None));
        match speed_policy_label(result) {
            "on" => entry.1 = Some(result),
            _ => entry.0 = Some(result),
        }
    }
    by_fixture
        .into_iter()
        .filter_map(|(fixture_id, (baseline, speed))| {
            let baseline = baseline?;
            let speed = speed?;
            Some(SpeedPolicyComparison {
                fixture_id,
                baseline_wall_ms: metric_value(baseline, "wall_time_ms"),
                speed_wall_ms: metric_value(speed, "wall_time_ms"),
                baseline_model_calls: metric_value(baseline, "model_calls"),
                speed_model_calls: metric_value(speed, "model_calls"),
                baseline_tool_calls: metric_value(baseline, "tool_calls"),
                speed_tool_calls: metric_value(speed, "tool_calls"),
                baseline_child_tasks: metric_value(baseline, "child_task_count"),
                speed_child_tasks: metric_value(speed, "child_task_count"),
                quality: if baseline.report.outcome == speed.report.outcome {
                    format!("matched `{:?}`", speed.report.outcome)
                } else {
                    format!(
                        "changed `{:?}` -> `{:?}`",
                        baseline.report.outcome, speed.report.outcome
                    )
                },
            })
        })
        .collect()
}

fn verification_remaining_gaps(events: &[crate::EvalTrajectoryEvent]) -> String {
    events
        .iter()
        .rev()
        .find(|event| event.event_type == "verification_completed" && event.is_error)
        .map(|_| "see failure message or verification trace".to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn pass_rate_rows(report: &EvalSuiteReport) -> Vec<(String, usize, usize)> {
    let mut rows = BTreeMap::<String, (usize, usize)>::new();
    for result in &report.results {
        let passed = usize::from(result.report.outcome == EvalOutcome::Pass);
        let model_scope = format!("{}/{}", result.report.run.provider, result.report.run.model);
        let entry = rows.entry(format!("model:{model_scope}")).or_insert((0, 0));
        entry.0 += passed;
        entry.1 += 1;
        for tag in result
            .report
            .run
            .tags
            .iter()
            .filter(|tag| tag.starts_with("tool:"))
        {
            let entry = rows.entry(tag.clone()).or_insert((0, 0));
            entry.0 += passed;
            entry.1 += 1;
        }
    }
    rows.into_iter()
        .map(|(scope, (passed, total))| (scope, passed, total))
        .collect()
}

fn failure_groups(report: &EvalSuiteReport) -> BTreeMap<(String, String, String), usize> {
    let mut groups = BTreeMap::new();
    for result in &report.results {
        if result.report.outcome == EvalOutcome::Pass {
            continue;
        }
        let tool = result
            .report
            .run
            .tags
            .iter()
            .find_map(|tag| tag.strip_prefix("tool:"))
            .unwrap_or("unknown")
            .to_string();
        let model = format!("{}/{}", result.report.run.provider, result.report.run.model);
        let class = result
            .report
            .failure_class
            .as_ref()
            .map(|class| format!("{class:?}"))
            .unwrap_or_else(|| "Unknown".to_string());
        *groups.entry((tool, model, class)).or_insert(0) += 1;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{
        ContextAssemblyCompleted, ContextEntrypointCandidatesInjected, InferenceStarted,
        RoderEvent, ToolCallCompleted, ToolCallRequested, ToolOutputTruncated,
        VerificationCompleted, VerificationRequired,
    };
    use roder_api::tasks::TaskStarted;

    #[test]
    fn context_eval_metrics_track_budget_entrypoints_and_truncation_follow_up() {
        let events = vec![
            RoderEvent::ContextAssemblyCompleted(ContextAssemblyCompleted {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                block_count: 1,
                total_byte_count: 800,
                estimated_tokens: 200,
                prompt_estimated_tokens: 200,
                token_budget: Some(1_000),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::ContextEntrypointCandidatesInjected(ContextEntrypointCandidatesInjected {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                candidate_count: 3,
                block_byte_count: 120,
                estimated_tokens: 30,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::ToolOutputTruncated(ToolOutputTruncated {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                tool_id: "tool-a".to_string(),
                tool_name: Some("grep".to_string()),
                original_line_count: 1_000,
                original_char_count: 40_000,
                inline_char_count: 2_000,
                artifact_backed: false,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::ToolCallRequested(ToolCallRequested {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                tool_id: "tool-b".to_string(),
                tool_name: "grep".to_string(),
                display_payload: None,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ];

        let metrics = eval_metrics(&events, 42, &EvalOutcome::Pass);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap()
        };

        assert_eq!(value("context_estimated_tokens"), 200.0);
        assert_eq!(value("context_bytes"), 800.0);
        assert_eq!(value("entrypoint_candidates"), 3.0);
        assert_eq!(value("entrypoint_injection_event"), 2.0);
        assert_eq!(value("truncation_follow_ups"), 1.0);
        assert_eq!(value("tool_output_truncations"), 1.0);
    }

    #[test]
    fn verification_eval_metrics_track_required_completed_and_gaps() {
        let events = vec![
            RoderEvent::VerificationRequired(VerificationRequired {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                reason: "code_changes_without_verification".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                tool_evidence: vec!["write_file: wrote src/lib.rs".to_string()],
                tests_run: Vec::new(),
                open_gaps: Vec::new(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::VerificationCompleted(VerificationCompleted {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                passed: false,
                changed_files: vec!["src/lib.rs".to_string()],
                tool_evidence: vec!["write_file: wrote src/lib.rs".to_string()],
                tests_run: Vec::new(),
                open_gaps: vec!["tests not run".to_string()],
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ];

        let metrics = eval_metrics(&events, 42, &EvalOutcome::Fail);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap()
        };

        assert_eq!(value("verification_required"), 1.0);
        assert_eq!(value("verification_completed"), 0.0);
        assert_eq!(value("verification_failed"), 1.0);
        assert_eq!(value("verification_open_gaps"), 1.0);
    }

    #[test]
    fn search_eval_metrics_track_grep_engine_and_latency_metadata() {
        let events = vec![RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: "grep-a".to_string(),
            tool_name: Some("grep".to_string()),
            display_payload: Some(serde_json::json!({
                "query": "BUG_ROOT_CAUSE_TOKEN",
                "engine": "indexed",
                "candidate_files": 4,
                "verified_files": 2,
                "elapsed_ms": 7,
                "index_bytes": 4096,
                "index_build_time_ms": 3
            })),
            is_error: false,
            output: None,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        })];

        let metrics = eval_metrics(&events, 42, &EvalOutcome::Pass);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap()
        };

        assert_eq!(value("grep_calls"), 1.0);
        assert_eq!(value("grep_indexed_calls"), 1.0);
        assert_eq!(value("grep_candidate_files"), 4.0);
        assert_eq!(value("grep_verified_files"), 2.0);
        assert_eq!(value("grep_elapsed_ms"), 7.0);
        assert_eq!(value("grep_index_bytes"), 4096.0);
        assert_eq!(value("grep_index_build_time_ms"), 3.0);
    }

    #[test]
    fn speed_eval_metrics_track_child_tasks_and_deadline_remaining() {
        let events = vec![
            RoderEvent::InferenceStarted(InferenceStarted {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                engine_id: "mock".to_string(),
                speed_policy: None,
                deadline_remaining_seconds: Some(27),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
            RoderEvent::TaskStarted(TaskStarted {
                task_id: "task-a".to_string(),
                executor_id: "subagent".to_string(),
                task_kind: "subagent".to_string(),
                queue_depth: 0,
                thread_id: Some("thread-a".to_string()),
                turn_id: Some("turn-a".to_string()),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ];

        let metrics = eval_metrics(&events, 42, &EvalOutcome::Pass);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap()
        };

        assert_eq!(value("model_calls"), 1.0);
        assert_eq!(value("child_task_count"), 1.0);
        assert_eq!(value("deadline_remaining_seconds"), 27.0);
    }
}
