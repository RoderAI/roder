//! Repeats of the live tier, what they add up to per task, and the saved
//! baseline they are compared with.
//!
//! A task run once says little about a hosted model: it passed or failed on
//! one draw. `JEV_EVAL_N` runs each task that many times, the tally counts
//! `verdict_ok`, `truth_ok` and false greens per task, and a baseline saved
//! from an earlier run (with the [`Pin`] it was taken at) turns "49 of 52" into
//! "these tasks got worse". The comparison is on rates, so a baseline of
//! three runs still stands against a run of five. It is all offline logic;
//! the live tier only feeds it rows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::grade::Marks;
use super::pin::{BASELINE_FILE, Pin};
use super::rows::Row;

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
    pub(crate) input_tokens: Option<u64>,
}

impl<'a> From<&'a Row> for Run<'a> {
    fn from(row: &'a Row) -> Self {
        Self {
            task: &row.task,
            marks: row.marks,
            steps: row.steps,
            input_tokens: row.input_tokens,
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
    /// Over the runs that reported tokens; absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) median_input_tokens: Option<f64>,
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
                median_input_tokens: median(
                    runs.iter()
                        .filter_map(|run| run.input_tokens.map(|tokens| tokens as f64))
                        .collect(),
                ),
            };
            (task.to_string(), tally)
        })
        .collect()
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
        "{:<26} {:>4} {:>5} {:>7} {:>5} {:>11} {:>6} {:>13}\n",
        "task", "runs", "pass", "verdict", "truth", "false_green", "steps", "input_tokens"
    );
    for (task, tally) in tally {
        out.push_str(&format!(
            "{:<26} {:>4} {:>5} {:>7} {:>5} {:>11} {:>6} {:>13}\n",
            task,
            tally.runs,
            tally.passed,
            tally.verdict_ok,
            tally.truth_ok,
            tally.false_green,
            tally.median_steps,
            tally
                .median_input_tokens
                .map_or("-".to_string(), |tokens| format!("{tokens}")),
        ));
    }
    out
}

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
    /// fails on a task getting worse, since some tasks are known to fail;
    /// with none, on any run that failed.
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
            Some(comparison) => failures.extend(
                comparison
                    .regressions
                    .iter()
                    .map(|regression| format!("worse than the baseline: {regression}")),
            ),
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
        if !self.drift.is_empty() {
            out.push_str(&format!(
                "MID-RUN EDIT: {}. These numbers describe no one tree.\n",
                self.drift.join("; ")
            ));
        }
        if let Some(comparison) = &self.comparison {
            for (heading, lines) in [
                ("REGRESSION", &comparison.regressions),
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
            if comparison.regressions.is_empty() {
                out.push_str("no regressions vs baseline\n");
            }
        }
        for reason in &self.refused {
            out.push_str(&format!("baseline not saved: {reason}\n"));
        }
        out
    }
}
