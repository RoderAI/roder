//! The saved baseline a live run is compared with, and the comparison: the
//! rates that make a task worse, and the work bounds that make it heavier.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{Tally, TaskTally};
use crate::fixture_harness::evals::pin::Pin;

/// How far a task's median work may rise over the baseline's before the
/// task counts as doing more work: a quarter, and for call counts at least
/// one whole call, since a median over three runs moves in whole calls.
pub(crate) const WORK_BOUND: f64 = 0.25;

/// The numbers of an earlier run, and the pin they were taken at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Baseline {
    pub(crate) pin: Pin,
    /// Runs per task.
    pub(crate) n: usize,
    pub(crate) tasks: Tally,
}

impl Baseline {
    /// `None` when there is no file; a file that does not parse is an error,
    /// not an absent baseline.
    pub(crate) fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        serde_json::from_str(&raw)
            .map(Some)
            .with_context(|| format!("parse {}", path.display()))
    }

    pub(crate) fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
    }

    /// This baseline against a run whose pin at its start was `pin`.
    pub(crate) fn compare(&self, now: &Tally, pin: &Pin) -> Comparison {
        let mut comparison = Comparison::default();
        for (name, was, is) in [
            ("corpus", &self.pin.corpus, &pin.corpus),
            ("model", &self.pin.model, &pin.model),
            ("setup", &self.pin.setup, &pin.setup),
        ] {
            if was != is {
                comparison.notes.push(format!(
                    "{name} differs from the baseline's ({was} -> {is}); the counts may not compare"
                ));
            }
        }
        for (task, is) in now {
            let Some(was) = self.tasks.get(task) else {
                comparison.new.push(task.clone());
                continue;
            };
            let heavier = heavier(was, is);
            if !heavier.is_empty() {
                comparison
                    .work
                    .push(format!("{task}: {}", heavier.join(", ")));
            }
            let worse = worse(was, is);
            if !worse.is_empty() {
                comparison
                    .regressions
                    .push(format!("{task}: {}", worse.join(", ")));
            } else if is.passed * was.runs > was.passed * is.runs {
                comparison.improved.push(format!(
                    "{task}: passed {}/{} -> {}/{}",
                    was.passed, was.runs, is.passed, is.runs
                ));
            }
        }
        comparison
    }
}

/// How a task's rates got worse since the baseline; empty when they did not.
/// Rates, compared across as products, so runs of unequal count compare.
fn worse(was: &TaskTally, is: &TaskTally) -> Vec<String> {
    let mut worse = Vec::new();
    for (name, before, now) in [
        ("passed", was.passed, is.passed),
        ("verdict_ok", was.verdict_ok, is.verdict_ok),
        ("truth_ok", was.truth_ok, is.truth_ok),
    ] {
        if now * was.runs < before * is.runs {
            worse.push(format!("{name} {before}/{} -> {now}/{}", was.runs, is.runs));
        }
    }
    if is.false_green * was.runs > was.false_green * is.runs {
        worse.push(format!(
            "false_green {}/{} -> {}/{}",
            was.false_green, was.runs, is.false_green, is.runs
        ));
    }
    worse
}

/// How a task's work rose past [`WORK_BOUND`] since the baseline; empty
/// when it did not. A task that passes more often than it did may need more
/// work to do so, so its work is not held against it; a figure either side
/// lacks is not compared.
fn heavier(was: &TaskTally, is: &TaskTally) -> Vec<String> {
    if is.passed * was.runs > was.passed * is.runs {
        return Vec::new();
    }
    let rose = |before: Option<f64>, now: Option<f64>, slack: f64| match (before, now) {
        (Some(before), Some(now)) => now > before * (1.0 + WORK_BOUND) && now - before >= slack,
        _ => false,
    };
    let mut heavier = Vec::new();
    for (name, before, now, slack) in [
        (
            "model_calls",
            was.median_model_calls,
            is.median_model_calls,
            1.0,
        ),
        (
            "text_calls",
            was.median_text_calls,
            is.median_text_calls,
            1.0,
        ),
        (
            "input_tokens",
            was.median_input_tokens,
            is.median_input_tokens,
            0.0,
        ),
    ] {
        if rose(before, now, slack) {
            heavier.push(format!(
                "median {name} {} -> {}",
                before.unwrap_or_default(),
                now.unwrap_or_default()
            ));
        }
    }
    if rose(was.mean_cost, is.mean_cost, 0.0) {
        heavier.push(format!(
            "mean cost ${:.6} -> ${:.6}",
            was.mean_cost.unwrap_or_default(),
            is.mean_cost.unwrap_or_default()
        ));
    }
    heavier
}

/// A run set against a baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Comparison {
    /// Tasks whose pass, verdict, truth or false-green rate got worse.
    pub(crate) regressions: Vec<String>,
    /// Tasks whose median calls, tokens or mean cost rose past
    /// [`WORK_BOUND`]; these fail `JEV_EVAL_STRICT=1` as regressions do.
    pub(crate) work: Vec<String>,
    pub(crate) improved: Vec<String>,
    /// Tasks the baseline has no numbers for.
    pub(crate) new: Vec<String>,
    /// Ways the two runs were not measuring the same thing.
    pub(crate) notes: Vec<String>,
}
