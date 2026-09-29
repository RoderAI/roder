//! The eval tiers' rows: one JSONL line per task, graded on Jev alone and,
//! after a fallback, on the call's end state, and the table printed at the
//! end of a run.

use std::path::PathBuf;

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use super::{Expect, Outcome, Task};

/// One JSONL line per task.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Row {
    pub(crate) task: String,
    pub(crate) covers: String,
    pub(crate) tier: &'static str,
    pub(crate) variants: Vec<String>,
    pub(crate) pass: bool,
    pub(crate) failures: Vec<String>,
    pub(crate) status: Value,
    pub(crate) stopped_because: Option<String>,
    pub(crate) steps: usize,
    pub(crate) model_calls: usize,
    pub(crate) text_calls: usize,
    pub(crate) wall_ms: u64,
    pub(crate) url: String,
    /// Tier-specific telemetry, such as per-head confidence on live runs.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub(crate) telemetry: Value,
    /// With a fallback model: whether the call passes after it, and what the
    /// fallback cost. `pass` above is Jev alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback: Option<FallbackRow>,
}

/// The call graded after the fallback, and the fallback's own cost.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FallbackRow {
    /// Jev and the fallback together pass (Jev alone's pass when it did
    /// not run).
    pub(crate) pass: bool,
    pub(crate) ran: bool,
    pub(crate) failures: Vec<String>,
    pub(crate) status: Value,
    pub(crate) model: String,
    pub(crate) actions: usize,
    pub(crate) model_calls: usize,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) stopped_because: Option<String>,
    /// Each tool call: tool, arguments, what it pressed, the result's first
    /// line.
    pub(crate) trace: Vec<String>,
}

impl FallbackRow {
    /// Grade `outcome` after its fallback against `expect` (the task's
    /// fallback expectation, else its own), or carry Jev alone's pass.
    pub(crate) fn grade(task: &Task, outcome: &Outcome, jev_pass: bool) -> Self {
        let Some((after, fallback)) = outcome.after.as_deref() else {
            return Self {
                pass: jev_pass,
                ran: false,
                failures: Vec::new(),
                status: Value::Null,
                model: String::new(),
                actions: 0,
                model_calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                elapsed_ms: 0,
                stopped_because: None,
                trace: Vec::new(),
            };
        };
        // A task written for Jev alone is graded on its outcome after the
        // fallback, not on the wording of why Jev stopped: whether the call
        // stopped with a reason is the fallback's to say.
        let expect = match &task.fallback {
            Some(fallback) => fallback.expect.clone(),
            None => Expect {
                stopped: None,
                stopped_because_contains: None,
                ..task.expect.clone()
            },
        };
        let failures = expect.grade(after);
        Self {
            pass: failures.is_empty(),
            ran: true,
            failures,
            status: serde_json::to_value(fallback.status).unwrap_or(Value::Null),
            model: fallback.model.clone(),
            actions: fallback.actions.len(),
            model_calls: fallback.model_calls,
            input_tokens: fallback.usage.input_tokens,
            output_tokens: fallback.usage.output_tokens,
            elapsed_ms: fallback.elapsed_ms,
            stopped_because: fallback.stopped_because.clone(),
            trace: fallback
                .actions
                .iter()
                .map(|action| {
                    format!(
                        "{} {} [{}] -> {}",
                        action.tool,
                        action.args,
                        action.target.clone().unwrap_or_default(),
                        action.result
                    )
                })
                .collect(),
        }
    }
}

impl Row {
    pub(crate) fn new(
        task: &Task,
        tier: &'static str,
        variants: Vec<String>,
        outcome: &Outcome,
        failures: Vec<String>,
    ) -> Self {
        let result = &outcome.result;
        Self {
            task: task.id.clone(),
            covers: task.covers.clone(),
            tier,
            variants,
            pass: failures.is_empty(),
            failures,
            status: serde_json::to_value(result.status).unwrap_or(Value::Null),
            stopped_because: result.stopped_because.clone(),
            steps: result.actions.len(),
            model_calls: result.model_calls,
            text_calls: result.text_calls,
            wall_ms: outcome.wall_ms,
            url: result.url.clone(),
            telemetry: Value::Null,
            fallback: None,
        }
    }

    /// A row for a task that could not run at all.
    pub(crate) fn errored(task: &Task, tier: &'static str, error: &anyhow::Error) -> Self {
        Self {
            task: task.id.clone(),
            covers: task.covers.clone(),
            tier,
            variants: Vec::new(),
            pass: false,
            failures: vec![format!("run failed: {error:#}")],
            status: Value::Null,
            stopped_because: None,
            steps: 0,
            model_calls: 0,
            text_calls: 0,
            wall_ms: 0,
            url: String::new(),
            telemetry: Value::Null,
            fallback: None,
        }
    }
}

/// Write rows to `target/jev-evals/<name>.jsonl` and return the path.
pub(crate) fn write_rows(name: &str, rows: &[Row]) -> anyhow::Result<PathBuf> {
    let dir = crate::fixture_harness::target_dir().join("jev-evals");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{name}.jsonl"));
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row)?);
        out.push('\n');
    }
    std::fs::write(&path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// The pass/fail table, in corpus order, for the test's stderr.
pub(crate) fn table(tasks: &[Task], rows: &[Row]) -> String {
    let mut out = format!(
        "{:<26} {:<5} {:<18} {:>5} {:>6} {:>8}  {}\n",
        "task", "pass", "status", "steps", "calls", "wall_ms", "failures"
    );
    for task in tasks {
        let Some(row) = rows.iter().find(|row| row.task == task.id) else {
            continue;
        };
        out.push_str(&format!(
            "{:<26} {:<5} {:<18} {:>5} {:>6} {:>8}  {}\n",
            row.task,
            if row.pass { "pass" } else { "FAIL" },
            row.status.as_str().unwrap_or("-"),
            row.steps,
            row.model_calls,
            row.wall_ms,
            row.failures.join("; ")
        ));
    }
    let passed = rows.iter().filter(|row| row.pass).count();
    out.push_str(&format!("{passed}/{} passed\n", rows.len()));
    let with = rows
        .iter()
        .filter_map(|row| row.fallback.as_ref())
        .collect::<Vec<_>>();
    if !with.is_empty() {
        let ran = with.iter().filter(|row| row.ran).collect::<Vec<_>>();
        out.push_str(&format!(
            "Jev alone {passed}/{n}; Jev + fallback {}/{n}. The fallback ran on {} tasks \
             ({} passed after it): {} tool calls, {} model calls, {:.1} s, {} input and {} \
             output tokens in all.\n",
            with.iter().filter(|row| row.pass).count(),
            ran.len(),
            ran.iter().filter(|row| row.pass).count(),
            ran.iter().map(|row| row.actions).sum::<usize>(),
            ran.iter().map(|row| row.model_calls).sum::<usize>(),
            ran.iter().map(|row| row.elapsed_ms).sum::<u64>() as f64 / 1000.0,
            ran.iter().map(|row| row.input_tokens).sum::<u64>(),
            ran.iter().map(|row| row.output_tokens).sum::<u64>(),
            n = rows.len(),
        ));
        for row in rows
            .iter()
            .filter(|row| row.fallback.as_ref().is_some_and(|f| f.ran))
        {
            let fallback = row.fallback.as_ref().unwrap();
            out.push_str(&format!(
                "  fallback {:<26} jev {:<5} after {:<5} {:<18} {:>2} calls {:>6} ms  {}\n",
                row.task,
                if row.pass { "pass" } else { "FAIL" },
                if fallback.pass { "pass" } else { "FAIL" },
                fallback.status.as_str().unwrap_or("-"),
                fallback.actions,
                fallback.elapsed_ms,
                fallback.failures.join("; ")
            ));
        }
    }
    out
}
