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
    ]
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
        "# Roder Eval Report\n\n- Suite: `{}`\n- Fixtures: {}\n- Passed: {}\n- Failed: {}\n\n| Fixture | Outcome | Failure class | Trace excerpt |\n| --- | --- | --- | --- |\n",
        report.suite_id,
        report.results.len(),
        passed,
        report.results.len().saturating_sub(passed)
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
    let groups = failure_groups(report);
    if !groups.is_empty() {
        text.push_str("\n## Failure Groups\n\n| Tool | Model | Failure class | Count |\n| --- | --- | --- | --- |\n");
        for ((tool, model, class), count) in groups {
            text.push_str(&format!("| `{tool}` | `{model}` | `{class}` | {count} |\n"));
        }
    }
    text
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
