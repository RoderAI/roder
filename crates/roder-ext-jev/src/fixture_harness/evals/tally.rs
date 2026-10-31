//! Repeats of the live tier, what they add up to per task, and the saved
//! baseline they are compared with.
//!
//! A task run once says little about a hosted model: it passed or failed on
//! one draw. `JEV_EVAL_N` runs each task that many times, the tally counts
//! `verdict_ok`, `truth_ok` and false greens per task, and a baseline saved
//! from an earlier run (with the [`Pin`] it was taken at) turns "49 of 52" into
//! "these tasks got worse". The comparison is on rates, so a baseline of
//! three runs still stands against a run of five.
//!
//! It also gates on the work a run did, as fastbrowse's release gate does
//! (CHANGELOG 0.6.0): a task whose median decision calls, text calls or
//! input tokens, or mean decision cost, rose past [`WORK_BOUND`](baseline::WORK_BOUND) over the
//! baseline's has done more work, even when it still passes. Wall time
//! moves with the machine's and the provider's load rather than with the
//! build, so it is recorded and never gates, and the part of it lost to
//! transient provider failures is reported on its own (see [`Work`]). It is
//! all offline logic; the live tier only feeds it rows.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::grade::Marks;
use super::pin::{BASELINE_FILE, Pin};
use super::rows::Row;

mod baseline;
mod work;
#[cfg(test)]
mod work_tests;
pub(crate) use baseline::{Baseline, Comparison};
pub(crate) use work::Work;
use work::number;

/// The most runs per task `JEV_EVAL_N` accepts: it multiplies a billed run.
pub(crate) const MAX_REPEATS: usize = 20;

/// Fewest runs per task a saved baseline stands on.
pub(crate) const MIN_BASELINE_RUNS: usize = 3;

/// `JEV_EVAL_N`: how many times each task runs. Absent or blank is 1; zero,
/// a word and a number past [`MAX_REPEATS`] are errors, so a typo cannot
/// quietly run once.
pub(crate) fn repeats(raw: Option<&str>) -> anyhow::Result<usize> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(1);
    };
    let n = raw
        .parse::<usize>()
        .with_context(|| format!("JEV_EVAL_N must be a whole number, not {raw:?}"))?;
    if !(1..=MAX_REPEATS).contains(&n) {
        bail!("JEV_EVAL_N must be 1 to {MAX_REPEATS}, not {n}");
    }
    Ok(n)
}

/// Every task once before any twice, so a service or a machine that drifts
/// over the run spreads over the tasks instead of piling onto the last.
pub(crate) fn plan<T>(tasks: &[T], n: usize) -> Vec<(&T, usize)> {
    (1..=n)
        .flat_map(|repeat| tasks.iter().map(move |task| (task, repeat)))
        .collect()
}

/// Where the baseline lives: the file `JEV_EVAL_BASELINE` names, else
/// `tests/fixtures/evals/live-baseline.json` in this crate.
pub(crate) fn baseline_path(named: Option<&str>) -> PathBuf {
    match named.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/evals")
            .join(BASELINE_FILE),
    }
}

/// What the tally reads of one run of one task.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run<'a> {
    pub(crate) task: &'a str,
    pub(crate) marks: Marks,
    pub(crate) steps: usize,
    /// Decision calls the run made.
    pub(crate) model_calls: usize,
    /// Text-helper calls the run made.
    pub(crate) text_calls: usize,
    pub(crate) input_tokens: Option<u64>,
    /// Dollars the run's decisions cost, when the host reported it.
    pub(crate) cost: Option<f64>,
    /// The model that answered, as the host reported it.
    pub(crate) model: Option<&'a str>,
    pub(crate) wall_ms: u64,
    /// Of `wall_ms`, what transient provider failures cost, where a hosted
    /// transport ran.
    pub(crate) transient_ms: Option<u64>,
}

impl<'a> From<&'a Row> for Run<'a> {
    fn from(row: &'a Row) -> Self {
        Self {
            task: &row.task,
            marks: row.marks,
            steps: row.steps,
            model_calls: row.model_calls,
            text_calls: row.text_calls,
            input_tokens: row.input_tokens,
            cost: row.decision_cost,
            model: row.decision_model.as_deref(),
            wall_ms: row.wall_ms,
            transient_ms: row.transient_ms,
        }
    }
}

/// One task over its runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskTally {
    pub(crate) runs: usize,
    /// Runs where the verdict and the truth were both ok.
    pub(crate) passed: usize,
    pub(crate) verdict_ok: usize,
    pub(crate) truth_ok: usize,
    pub(crate) false_green: usize,
    pub(crate) median_steps: f64,
    /// Decision calls per run; absent in a baseline saved before calls were
    /// tallied, which the work gate then leaves alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_model_calls: Option<f64>,
    /// Text-helper calls per run, as `median_model_calls`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_text_calls: Option<f64>,
    /// Over the runs that reported tokens; absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_input_tokens: Option<f64>,
    /// Mean decision dollars over the runs that reported a cost; absent
    /// when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mean_cost: Option<f64>,
    /// Recorded, never gated: it moves with load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_wall_ms: Option<f64>,
    /// Over the runs whose transport reported it; absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_transient_ms: Option<f64>,
}

/// Every task of a run, by id.
pub(crate) type Tally = BTreeMap<String, TaskTally>;

pub(crate) fn tally<'a>(runs: impl IntoIterator<Item = Run<'a>>) -> Tally {
    let mut by_task: BTreeMap<&str, Vec<Run>> = BTreeMap::new();
    for run in runs {
        by_task.entry(run.task).or_default().push(run);
    }
    by_task
        .into_iter()
        .map(|(task, runs)| {
            let count =
                |keep: fn(&Marks) -> bool| runs.iter().filter(|run| keep(&run.marks)).count();
            let tally = TaskTally {
                runs: runs.len(),
                passed: count(|marks| marks.passed()),
                verdict_ok: count(|marks| marks.verdict_ok),
                truth_ok: count(|marks| marks.truth_ok),
                false_green: count(|marks| marks.false_green),
                median_steps: median(runs.iter().map(|run| run.steps as f64).collect())
                    .unwrap_or_default(),
                median_model_calls: median(runs.iter().map(|run| run.model_calls as f64).collect()),
                median_text_calls: median(runs.iter().map(|run| run.text_calls as f64).collect()),
                median_input_tokens: median(
                    runs.iter()
                        .filter_map(|run| run.input_tokens.map(|tokens| tokens as f64))
                        .collect(),
                ),
                mean_cost: mean(runs.iter().filter_map(|run| run.cost).collect()),
                median_wall_ms: median(runs.iter().map(|run| run.wall_ms as f64).collect()),
                median_transient_ms: median(
                    runs.iter()
                        .filter_map(|run| run.transient_ms.map(|ms| ms as f64))
                        .collect(),
                ),
            };
            (task.to_string(), tally)
        })
        .collect()
}

fn mean(values: Vec<f64>) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

/// A run's tally as the rows of a table, one task a line.
pub(crate) fn table(tally: &Tally) -> String {
    let mut out = format!(
        "{:<26} {:>4} {:>5} {:>7} {:>5} {:>11} {:>6} {:>6} {:>5} {:>13} {:>10} {:>8} {:>9}\n",
        "task",
        "runs",
        "pass",
        "verdict",
        "truth",
        "false_green",
        "steps",
        "calls",
        "text",
        "input_tokens",
        "mean_cost",
        "wall_ms",
        "transient"
    );
    for (task, tally) in tally {
        out.push_str(&format!(
            "{:<26} {:>4} {:>5} {:>7} {:>5} {:>11} {:>6} {:>6} {:>5} {:>13} {:>10} {:>8} {:>9}\n",
            task,
            tally.runs,
            tally.passed,
            tally.verdict_ok,
            tally.truth_ok,
            tally.false_green,
            tally.median_steps,
            number(tally.median_model_calls),
            number(tally.median_text_calls),
            number(tally.median_input_tokens),
            tally
                .mean_cost
                .map_or("-".to_string(), |dollars| format!("${dollars:.6}")),
            number(tally.median_wall_ms),
            number(tally.median_transient_ms),
        ));
    }
    out
}

/// What the environment asked of this run.
pub(crate) struct Settings {
    /// Runs per task.
    pub(crate) n: usize,
    /// The run covers part of the corpus (`JEV_EVAL_TASKS`).
    pub(crate) subset: bool,
    /// `JEV_EVAL_SAVE_BASELINE=1`.
    pub(crate) save: bool,
    /// `JEV_EVAL_STRICT=1`: with `save`, a run the strict gate would reject is
    /// not saved.
    pub(crate) strict: bool,
}

/// A live run once its rows are in.
#[derive(Debug, Serialize)]
pub(crate) struct Conclusion {
    pub(crate) n: usize,
    pub(crate) start: Pin,
    pub(crate) end: Pin,
    /// What moved between the two reads of the pin: a mid-run edit.
    pub(crate) drift: Vec<String>,
    pub(crate) tasks: Tally,
    /// The run's work across every task.
    pub(crate) work: Work,
    /// Decision dollars summed over the runs that reported a cost, and how
    /// many did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) decision_cost: Option<f64>,
    pub(crate) costed_runs: usize,
    /// Every model the host reported answering, such as the dated version
    /// behind `jev-latest`.
    pub(crate) served_models: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comparison: Option<Comparison>,
    /// Why a requested baseline was not saved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) refused: Vec<String>,
    /// The baseline to write, when one was asked for and may be.
    #[serde(skip)]
    pub(crate) save: Option<Baseline>,
    #[serde(skip)]
    failed_runs: usize,
    #[serde(skip)]
    false_green_runs: usize,
}

/// Tally `runs`, compare them with `saved` and decide whether they may
/// become the next baseline. `start` and `end` are the pin read before the
/// first task and after the last.
pub(crate) fn conclude(
    runs: &[Run],
    start: Pin,
    end: Pin,
    settings: &Settings,
    saved: Option<&Baseline>,
) -> Conclusion {
    let tasks = tally(runs.iter().copied());
    let costs = runs.iter().filter_map(|run| run.cost).collect::<Vec<_>>();
    let drift = start.drift(&end);
    let comparison = saved.map(|saved| saved.compare(&tasks, &start));
    let mut refused = Vec::new();
    if settings.save {
        if !drift.is_empty() {
            refused.push(format!(
                "the tree changed during the run ({}); its numbers describe no one tree",
                drift.join("; ")
            ));
        }
        if settings.n < MIN_BASELINE_RUNS {
            refused.push(format!(
                "a baseline needs JEV_EVAL_N of at least {MIN_BASELINE_RUNS}, not {}",
                settings.n
            ));
        }
        if settings.subset {
            refused.push(
                "JEV_EVAL_TASKS narrowed the run, and a baseline of part of the corpus would \
                 replace the baseline of all of it"
                    .into(),
            );
        }
    }
    let mut conclusion = Conclusion {
        n: settings.n,
        start,
        end,
        drift,
        tasks,
        work: Work::of(runs),
        decision_cost: costs.iter().copied().reduce(|a, b| a + b),
        costed_runs: costs.len(),
        served_models: runs
            .iter()
            .filter_map(|run| run.model.map(str::to_string))
            .collect(),
        comparison,
        refused,
        save: None,
        failed_runs: runs.iter().filter(|run| !run.marks.passed()).count(),
        false_green_runs: runs.iter().filter(|run| run.marks.false_green).count(),
    };
    // A run the strict gate rejects is not the next baseline: the gate would
    // fail the test after the file was written, and every later run would
    // compare itself with the numbers that were rejected.
    if settings.save && settings.strict && conclusion.refused.is_empty() {
        let failures = conclusion.strict_failures();
        if !failures.is_empty() {
            conclusion.refused.push(format!(
                "the run fails JEV_EVAL_STRICT=1 ({}), and a baseline is not saved from a run the \
                 gate rejects; run without JEV_EVAL_STRICT to accept it",
                failures.join("; ")
            ));
        }
    }
    conclusion.save = (settings.save && conclusion.refused.is_empty()).then(|| Baseline {
        pin: conclusion.start.clone(),
        n: settings.n,
        tasks: conclusion.tasks.clone(),
    });
    conclusion
}

impl Conclusion {
    /// Why `JEV_EVAL_STRICT=1` fails this run; empty means it holds. A
    /// mid-run edit, a false green and a run that produced no rows always
    /// fail it. Against a baseline it
    /// fails on a task getting worse, since some tasks are known to fail,
    /// and on a task doing more work than [`WORK_BOUND`](baseline::WORK_BOUND) allows over the
    /// baseline's; with none, on any run that failed.
    pub(crate) fn strict_failures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if self.tasks.is_empty() {
            failures.push("no run finished".to_string());
        }
        if !self.drift.is_empty() {
            failures.push(format!(
                "the tree changed mid-run: {}",
                self.drift.join("; ")
            ));
        }
        if self.false_green_runs > 0 {
            failures.push(format!("{} false green runs", self.false_green_runs));
        }
        match &self.comparison {
            Some(comparison) => {
                failures.extend(
                    comparison
                        .regressions
                        .iter()
                        .map(|regression| format!("worse than the baseline: {regression}")),
                );
                failures.extend(
                    comparison
                        .work
                        .iter()
                        .map(|work| format!("more work than the baseline: {work}")),
                );
            }
            None if self.failed_runs > 0 => {
                failures.push(format!("{} failed runs", self.failed_runs));
            }
            None => {}
        }
        failures
    }

    /// The pin, the tally and what the baseline made of it, for stderr.
    pub(crate) fn report(&self) -> String {
        let pin = &self.start;
        let mut out = format!(
            "pin: commit {}, worktree {}, corpus {}, model {}, setup {}\n{}",
            pin.commit,
            pin.worktree,
            pin.corpus,
            pin.model,
            pin.setup,
            table(&self.tasks)
        );
        out.push_str(&self.work.line());
        if !self.served_models.is_empty() {
            out.push_str(&format!(
                "served by: {}\n",
                self.served_models
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(dollars) = self.decision_cost {
            out.push_str(&format!(
                "decision cost: ${dollars:.6} over {} runs\n",
                self.costed_runs
            ));
        }
        if !self.drift.is_empty() {
            out.push_str(&format!(
                "MID-RUN EDIT: {}. These numbers describe no one tree.\n",
                self.drift.join("; ")
            ));
        }
        if let Some(comparison) = &self.comparison {
            for (heading, lines) in [
                ("REGRESSION", &comparison.regressions),
                ("MORE WORK", &comparison.work),
                ("improved", &comparison.improved),
                ("note", &comparison.notes),
            ] {
                for line in lines {
                    out.push_str(&format!("{heading} vs baseline: {line}\n"));
                }
            }
            if !comparison.new.is_empty() {
                out.push_str(&format!("no baseline for: {}\n", comparison.new.join(", ")));
            }
            if comparison.regressions.is_empty() && comparison.work.is_empty() {
                out.push_str("no regressions vs baseline\n");
            }
        }
        for reason in &self.refused {
            out.push_str(&format!("baseline not saved: {reason}\n"));
        }
        out
    }
}
