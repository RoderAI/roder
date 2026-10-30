//! The saved baseline a live run is compared with, and the comparison.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{Tally, TaskTally};
use crate::fixture_harness::evals::pin::Pin;

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

/// A run set against a baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Comparison {
    /// Tasks whose pass, verdict, truth or false-green rate got worse.
    pub(crate) regressions: Vec<String>,
    pub(crate) improved: Vec<String>,
    /// Tasks the baseline has no numbers for.
    pub(crate) new: Vec<String>,
    /// Ways the two runs were not measuring the same thing.
    pub(crate) notes: Vec<String>,
}
