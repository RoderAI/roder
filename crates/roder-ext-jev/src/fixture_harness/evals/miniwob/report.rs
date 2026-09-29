//! MiniWoB++ rows, their failure causes, and the printed summary.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::episode::{EPISODE_ENDED, Episode, STEP_CAP};
use super::{Manifest, TaskEntry};
use crate::engine::JevStatus;

/// One JSONL line per episode.
#[derive(Debug, Clone, Serialize)]
pub(super) struct Row {
    pub(super) task: String,
    pub(super) seed: u64,
    pub(super) supported: bool,
    /// Whether the episode ran; an unsupported task runs only on request.
    pub(super) attempted: bool,
    /// `raw_reward > 0`, BrowserGym's convention, at the episode's end
    /// (after a fallback, when one ran).
    pub(super) success: bool,
    /// The same when Jev stopped, before any fallback.
    pub(super) jev_success: bool,
    /// The fallback that followed Jev, when one ran: its status, steps,
    /// model calls, tokens and time.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub(super) fallback: Value,
    pub(super) raw_reward: Option<f64>,
    /// Time-penalised; kept for reference, not comparable across harnesses.
    pub(super) reward: Option<f64>,
    pub(super) reward_reason: Option<String>,
    pub(super) episode_done: bool,
    /// Jev's status, or `episode_ended` / `step_cap` when the harness
    /// stopped the run.
    pub(super) jev_status: String,
    pub(super) stopped_because: Option<String>,
    pub(super) cause: String,
    pub(super) steps: usize,
    pub(super) model_calls: usize,
    pub(super) text_calls: usize,
    pub(super) wall_ms: u64,
    pub(super) goal: String,
    pub(super) decision_model: Option<String>,
    pub(super) text_model: String,
    /// Each text-helper call's latency, the summed usage, and each call's
    /// outcome; never the values.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub(super) text: Value,
    /// Executed actions, then the final DONE or BLOCKED if Jev gave one.
    pub(super) trace: Vec<String>,
    pub(super) start_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

impl Row {
    fn base(task: &TaskEntry, seed: u64, text_model: &str) -> Self {
        Self {
            task: task.id.clone(),
            seed,
            supported: task.supported,
            attempted: false,
            success: false,
            jev_success: false,
            fallback: Value::Null,
            raw_reward: None,
            reward: None,
            reward_reason: None,
            episode_done: false,
            jev_status: String::new(),
            stopped_because: None,
            cause: String::new(),
            steps: 0,
            model_calls: 0,
            text_calls: 0,
            wall_ms: 0,
            goal: String::new(),
            decision_model: None,
            text_model: text_model.into(),
            text: Value::Null,
            trace: Vec::new(),
            start_actions: Vec::new(),
            error: None,
        }
    }

    pub(super) fn not_attempted(task: &TaskEntry, seed: u64, text_model: &str) -> Self {
        Self {
            jev_status: "not_attempted".into(),
            cause: "unsupported (not attempted)".into(),
            ..Self::base(task, seed, text_model)
        }
    }

    pub(super) fn errored(
        task: &TaskEntry,
        seed: u64,
        text_model: &str,
        error: &anyhow::Error,
    ) -> Self {
        Self {
            attempted: true,
            jev_status: "setup_error".into(),
            cause: "setup error".into(),
            error: Some(format!("{error:#}")),
            ..Self::base(task, seed, text_model)
        }
    }

    pub(super) fn ran(task: &TaskEntry, seed: u64, text_model: &str, episode: Episode) -> Self {
        let result = &episode.result;
        let jev_status = match result.stopped_because.as_deref() {
            Some(EPISODE_ENDED) => "episode_ended".to_string(),
            Some(STEP_CAP) => "step_cap".to_string(),
            _ => serde_json::to_value(result.status)
                .ok()
                .and_then(|status| status.as_str().map(str::to_string))
                .unwrap_or_default(),
        };
        let mut trace = result
            .actions
            .iter()
            .map(|action| {
                let mut line = format!("{} {}", action.kind, action.action);
                if let Some(text) = &action.text {
                    line.push_str(&format!(" = {text:?}"));
                }
                if action.page_changed == Some(false) {
                    line.push_str(" [unchanged]");
                }
                if action.covered {
                    line.push_str(" [covered]");
                }
                if let Some(refused) = &action.refused {
                    line.push_str(&format!(" [refused: {refused}]"));
                }
                line
            })
            .collect::<Vec<_>>();
        if matches!(result.status, JevStatus::Done | JevStatus::Blocked)
            && result.stopped_because.is_none()
            && let Some(last) = result.decisions.last()
            && matches!(last.choice.as_str(), "DONE" | "BLOCKED")
        {
            trace.push(format!("{} ({:.2})", last.choice, last.confidence));
        }
        let mut row = Self {
            attempted: true,
            jev_status,
            stopped_because: result.stopped_because.clone(),
            steps: result.actions.len(),
            model_calls: result.model_calls,
            text_calls: result.text_calls,
            wall_ms: episode.wall_ms,
            goal: episode.goal.clone(),
            decision_model: result.decisions.iter().rev().find_map(|d| d.model.clone()),
            trace,
            start_actions: episode.start_actions.clone(),
            ..Self::base(task, seed, text_model)
        };
        match &episode.grade {
            Ok(grade) => {
                row.raw_reward = Some(grade.raw_reward);
                row.reward = Some(grade.reward);
                row.reward_reason = grade.reason.clone();
                row.episode_done = grade.done;
                row.success = grade.done && grade.raw_reward > 0.0;
            }
            Err(error) => row.error = Some(format!("grade: {error}")),
        }
        row.jev_success = match &episode.jev_grade {
            Some(grade) => grade.done && grade.raw_reward > 0.0,
            None => row.success,
        };
        if let Some(fallback) = &episode.fallback {
            row.fallback = serde_json::json!({
                "status": fallback.status,
                "stopped_because": fallback.stopped_because,
                "actions": fallback.actions.len(),
                "model_calls": fallback.model_calls,
                "input_tokens": fallback.usage.input_tokens,
                "output_tokens": fallback.usage.output_tokens,
                "elapsed_ms": fallback.elapsed_ms,
                "model": fallback.model,
                "trace": fallback.actions.iter().map(|action| format!(
                    "{} {}{}",
                    action.tool,
                    action.target.clone().unwrap_or_default(),
                    if action.error { " [error]" } else { "" }
                )).collect::<Vec<_>>(),
            });
        }
        row.cause = cause(&row, result.actions.iter().map(|a| a.page_changed));
        row
    }
}

/// A coarse, automatic failure cause; the analysis refines it by hand.
fn cause(row: &Row, changed: impl DoubleEndedIterator<Item = Option<bool>>) -> String {
    if row.success {
        return "success".into();
    }
    if row.episode_done {
        return "task graded the episode a failure".into();
    }
    let stalled = {
        let last = changed.rev().take(3).collect::<Vec<_>>();
        last.len() == 3 && last.iter().all(|changed| *changed == Some(false))
    };
    match row.jev_status.as_str() {
        "done" => "Jev said DONE, task not complete",
        "blocked" if stalled => "stalled: three actions changed nothing",
        "blocked" => "Jev answered BLOCKED",
        "step_cap" | "budget_exceeded" => "decision cap reached",
        "timed_out" => "episode timed out",
        "needs_input" => "no text value (needs_input)",
        "unavailable" => "model unavailable",
        "episode_ended" => "task graded the episode a failure",
        _ if row.error.is_some() => "grade error",
        _ => "run error",
    }
    .into()
}

fn rate(success: usize, total: usize) -> String {
    if total == 0 {
        return "-".into();
    }
    format!(
        "{success}/{total} = {:.1}%",
        100.0 * success as f64 / total as f64
    )
}

/// The printed summary: headline scores, per-task rates, and causes.
pub(super) fn summary(manifest: &Manifest, tasks: &[&TaskEntry], rows: &[Row]) -> String {
    let count = |keep: &dyn Fn(&Row) -> bool| rows.iter().filter(|row| keep(row)).count();
    let official = count(&|row| row.success && row.supported);
    let attempted_success = count(&|row| row.success);
    let supported_rows = count(&|row| row.supported);
    let unsupported_attempted = count(&|row| !row.supported && row.attempted);
    let mut out = format!(
        "MiniWoB++ {} @ {} ({} tasks selected, {} excluded: {})\n",
        manifest.source,
        &manifest.commit[..8],
        tasks.len(),
        manifest.excluded.len(),
        manifest
            .excluded
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str(&format!(
        "overall (unsupported count as failures): {}\n",
        rate(official, rows.len())
    ));
    out.push_str(&format!(
        "overall as attempted (every episode's own reward): {}\n",
        rate(attempted_success, rows.len())
    ));
    out.push_str(&format!(
        "supported only: {}\n",
        rate(official, supported_rows)
    ));
    out.push_str(&format!(
        "unsupported, attempted: {}\n",
        rate(
            count(&|row| row.success && !row.supported),
            unsupported_attempted
        )
    ));
    let fell_back = rows
        .iter()
        .filter(|row| !row.fallback.is_null())
        .collect::<Vec<_>>();
    if !fell_back.is_empty() {
        let jev_alone = count(&|row| row.jev_success);
        let sum = |key: &str| {
            fell_back
                .iter()
                .map(|row| row.fallback[key].as_u64().unwrap_or(0))
                .sum::<u64>()
        };
        out.push_str(&format!(
            "Jev alone, every episode's own reward: {}; Jev + fallback: {}\n\
             Jev alone, unsupported as failures: {}; Jev + fallback: {}\n\
             the fallback ran on {} episodes and turned {} into successes: {} tool calls, {} \
             model calls, {:.1} s ({:.1} s per fallback), {} input and {} output tokens\n",
            rate(jev_alone, rows.len()),
            rate(attempted_success, rows.len()),
            rate(count(&|row| row.jev_success && row.supported), rows.len()),
            rate(official, rows.len()),
            fell_back.len(),
            fell_back
                .iter()
                .filter(|row| row.success && !row.jev_success)
                .count(),
            sum("actions"),
            sum("model_calls"),
            sum("elapsed_ms") as f64 / 1000.0,
            sum("elapsed_ms") as f64 / 1000.0 / fell_back.len() as f64,
            sum("input_tokens"),
            sum("output_tokens"),
        ));
    }
    let ran = rows.iter().filter(|row| row.attempted).collect::<Vec<_>>();
    let wall = ran.iter().map(|row| row.wall_ms).sum::<u64>();
    let steps = ran.iter().map(|row| row.steps).sum::<usize>();
    let calls = ran.iter().map(|row| row.model_calls).sum::<usize>();
    out.push_str(&format!(
        "{} episodes run, {:.1} s summed episode wall, {:.1} s mean per episode, \
         {steps} steps ({:.2} s per step), {calls} model calls\n\n",
        ran.len(),
        wall as f64 / 1000.0,
        wall as f64 / 1000.0 / ran.len().max(1) as f64,
        wall as f64 / 1000.0 / steps.max(1) as f64,
    ));
    out.push_str(&format!(
        "{:<30} {:<5} {:>7} {:>7} {:>6} {:>7}  {}\n",
        "task", "supp", "jev", "success", "steps", "mean_s", "causes"
    ));
    for task in tasks {
        let task_rows = rows
            .iter()
            .filter(|row| row.task == task.id)
            .collect::<Vec<_>>();
        let won = task_rows.iter().filter(|row| row.success).count();
        let jev_won = task_rows.iter().filter(|row| row.jev_success).count();
        let mut causes = BTreeMap::<&str, usize>::new();
        for row in &task_rows {
            if !row.success {
                *causes.entry(row.cause.as_str()).or_default() += 1;
            }
        }
        let n = task_rows.len().max(1);
        out.push_str(&format!(
            "{:<30} {:<5} {:>7} {:>7} {:>6.1} {:>7.1}  {}\n",
            task.id,
            if task.supported { "yes" } else { "no" },
            format!("{jev_won}/{}", task_rows.len()),
            format!("{won}/{}", task_rows.len()),
            task_rows.iter().map(|row| row.steps).sum::<usize>() as f64 / n as f64,
            task_rows.iter().map(|row| row.wall_ms).sum::<u64>() as f64 / 1000.0 / n as f64,
            causes
                .iter()
                .map(|(cause, n)| format!("{cause} x{n}"))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let mut causes = BTreeMap::<(&str, bool), usize>::new();
    for row in rows.iter().filter(|row| !row.success) {
        *causes
            .entry((row.cause.as_str(), row.supported))
            .or_default() += 1;
    }
    let mut causes = causes.into_iter().collect::<Vec<_>>();
    causes.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    out.push_str("\nfailure causes (episodes):\n");
    for ((cause, supported), n) in causes {
        let group = if supported {
            "supported"
        } else {
            "unsupported"
        };
        out.push_str(&format!("{n:>5}  {cause} ({group})\n"));
    }
    out
}
