use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roder_api::events::RoderEvent;
use roder_api::inference::InferenceEvent;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{EvalMetric, EvalMetricKind, EvalOutcome, EvalTrajectory, EvalTrajectoryEvent};

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
    reports.sort_by(|left, right| right.generated_at.cmp(&left.generated_at));
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
    vec![
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
    ]
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
        ContextAssemblyCompleted, ContextEntrypointCandidatesInjected, RoderEvent,
        ToolCallRequested, ToolOutputTruncated,
    };

    #[test]
    fn context_eval_metrics_track_budget_entrypoints_and_truncation_follow_up() {
        let events = vec![
            RoderEvent::ContextAssemblyCompleted(ContextAssemblyCompleted {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                block_count: 1,
                total_byte_count: 800,
                estimated_tokens: 200,
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
}
