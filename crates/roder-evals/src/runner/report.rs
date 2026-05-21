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
    text
}
