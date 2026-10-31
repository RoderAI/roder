//! The work a live run did over all its tasks, and what transient provider
//! failures took out of its wall time.

use serde::Serialize;

use super::{Run, median};

/// A median or other optional figure for a table, `-` when absent.
pub(super) fn number(value: Option<f64>) -> String {
    value.map_or("-".to_string(), |value| format!("{value}"))
}

/// The work a live run did over all its tasks: the figures the gate
/// compares per task, taken across the whole run, and its wall time with
/// what transient provider failures took out of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Work {
    pub(crate) runs: usize,
    pub(crate) median_steps: Option<f64>,
    pub(crate) median_model_calls: Option<f64>,
    pub(crate) median_text_calls: Option<f64>,
    pub(crate) model_calls: usize,
    pub(crate) text_calls: usize,
    /// Over the runs that reported tokens.
    pub(crate) input_tokens: u64,
    pub(crate) median_wall_ms: Option<f64>,
    /// Time lost to transient provider failures, summed, and how many runs
    /// lost any. Part of the wall time; never gated.
    pub(crate) transient_ms: u64,
    pub(crate) transient_runs: usize,
    /// Wall time less transient time, per run: what the build itself took.
    pub(crate) median_wall_less_transient_ms: Option<f64>,
}

impl Work {
    pub(crate) fn of(runs: &[Run]) -> Self {
        let all = |value: fn(&Run) -> f64| median(runs.iter().map(value).collect());
        Self {
            runs: runs.len(),
            median_steps: all(|run| run.steps as f64),
            median_model_calls: all(|run| run.model_calls as f64),
            median_text_calls: all(|run| run.text_calls as f64),
            model_calls: runs.iter().map(|run| run.model_calls).sum(),
            text_calls: runs.iter().map(|run| run.text_calls).sum(),
            input_tokens: runs.iter().filter_map(|run| run.input_tokens).sum(),
            median_wall_ms: all(|run| run.wall_ms as f64),
            transient_ms: runs.iter().filter_map(|run| run.transient_ms).sum(),
            transient_runs: runs
                .iter()
                .filter(|run| run.transient_ms.is_some_and(|ms| ms > 0))
                .count(),
            median_wall_less_transient_ms: all(|run| {
                run.wall_ms.saturating_sub(run.transient_ms.unwrap_or(0)) as f64
            }),
        }
    }

    /// The report's line for it.
    pub(crate) fn line(&self) -> String {
        format!(
            "work over {} runs: median steps {}, decision calls {} ({} in all), text calls {} \
             ({} in all), {} input tokens in all; median wall {} ms, {} ms less transient; {} ms \
             lost to transient provider failures over {} runs\n",
            self.runs,
            number(self.median_steps),
            number(self.median_model_calls),
            self.model_calls,
            number(self.median_text_calls),
            self.text_calls,
            self.input_tokens,
            number(self.median_wall_ms),
            number(self.median_wall_less_transient_ms),
            self.transient_ms,
            self.transient_runs,
        )
    }
}
