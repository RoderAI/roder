//! The work gate and the transient-time report, offline.

use super::super::grade::Marks;
use super::super::pin::Pin;
use super::{Baseline, Run, Settings, Work, conclude, tally};

fn pin() -> Pin {
    Pin {
        commit: "c0ffee".into(),
        worktree: "aaaa".into(),
        corpus: "bbbb".into(),
        model: "jev-latest".into(),
        setup: "task values".into(),
    }
}

/// A passing run that made `calls` decision calls on `tokens` input tokens.
fn run(task: &str, calls: usize, tokens: u64) -> Run<'_> {
    Run {
        task,
        marks: Marks::new(true, true),
        steps: calls.saturating_sub(1),
        model_calls: calls,
        text_calls: 0,
        input_tokens: Some(tokens),
        cost: Some(tokens as f64 * 1e-7),
        model: None,
        wall_ms: 2000,
        transient_ms: Some(0),
    }
}

fn baseline(runs: Vec<Run>) -> Baseline {
    Baseline {
        pin: pin(),
        n: 3,
        tasks: tally(runs),
    }
}

fn strict() -> Settings {
    Settings {
        n: 3,
        subset: false,
        save: false,
        strict: true,
    }
}

#[test]
fn a_tally_takes_the_median_work_of_each_task() {
    let mut runs = vec![run("a", 4, 1000), run("a", 6, 3000), run("a", 5, 2000)];
    runs[1].text_calls = 2;
    runs[1].transient_ms = Some(900);
    runs[2].wall_ms = 5000;
    let tally = tally(runs);
    let a = &tally["a"];
    assert_eq!(a.median_model_calls, Some(5.0));
    assert_eq!(a.median_text_calls, Some(0.0));
    assert_eq!(a.median_input_tokens, Some(2000.0));
    assert_eq!(a.median_wall_ms, Some(2000.0));
    assert_eq!(a.median_transient_ms, Some(0.0));
}

#[test]
fn more_calls_tokens_or_cost_than_the_bound_is_more_work() {
    let saved = baseline(vec![run("a", 4, 10_000); 3]);
    // Within the bound: a quarter more calls is not past it.
    let same = conclude(
        &[run("a", 5, 12_000); 3],
        pin(),
        pin(),
        &strict(),
        Some(&saved),
    );
    assert!(same.comparison.as_ref().unwrap().work.is_empty());
    assert!(
        same.strict_failures().is_empty(),
        "{:?}",
        same.strict_failures()
    );

    let heavier = conclude(
        &[run("a", 6, 14_000); 3],
        pin(),
        pin(),
        &strict(),
        Some(&saved),
    );
    let work = &heavier.comparison.as_ref().unwrap().work;
    assert_eq!(
        work,
        &[
            "a: median model_calls 4 -> 6, median input_tokens 10000 -> 14000, mean cost \
             $0.001000 -> $0.001400"
        ]
    );
    // It gates as a regression does, and the report names it.
    assert_eq!(
        heavier.strict_failures(),
        [format!("more work than the baseline: {}", work[0])]
    );
    assert!(heavier.report().contains("MORE WORK vs baseline: a:"));
}

#[test]
fn one_extra_call_on_a_short_task_is_within_the_bound_only_below_a_whole_call() {
    // 2 -> 3 calls is past a quarter and a whole call more.
    let saved = baseline(vec![run("a", 2, 1000); 3]);
    let now = conclude(
        &[run("a", 3, 1000); 3],
        pin(),
        pin(),
        &strict(),
        Some(&saved),
    );
    assert_eq!(
        now.comparison.unwrap().work,
        ["a: median model_calls 2 -> 3"]
    );
    // 2 -> 2.5, a median between runs, is not a whole call.
    let mut half = vec![run("a", 2, 1000), run("a", 3, 1000)];
    half.extend([run("a", 2, 1000), run("a", 3, 1000)]);
    let now = conclude(&half, pin(), pin(), &strict(), Some(&saved));
    assert!(now.comparison.unwrap().work.is_empty());
}

#[test]
fn a_task_that_passes_more_often_may_work_harder() {
    let mut failing = vec![run("a", 2, 1000); 3];
    for run in &mut failing {
        run.marks = Marks::new(false, false);
    }
    let saved = baseline(failing);
    let now = conclude(
        &[run("a", 8, 9000); 3],
        pin(),
        pin(),
        &strict(),
        Some(&saved),
    );
    let comparison = now.comparison.unwrap();
    assert!(comparison.work.is_empty(), "{comparison:?}");
    assert_eq!(comparison.improved, ["a: passed 0/3 -> 3/3"]);
}

#[test]
fn a_baseline_without_work_figures_is_not_gated_on_them() {
    let saved: Baseline = serde_json::from_value(serde_json::json!({
        "pin": pin(),
        "n": 3,
        "tasks": {"a": {
            "runs": 3, "passed": 3, "verdict_ok": 3, "truth_ok": 3, "false_green": 0,
            "median_steps": 1.0
        }}
    }))
    .unwrap();
    let now = conclude(
        &[run("a", 9, 9000); 3],
        pin(),
        pin(),
        &strict(),
        Some(&saved),
    );
    assert!(
        now.strict_failures().is_empty(),
        "{:?}",
        now.strict_failures()
    );
}

#[test]
fn transient_time_is_reported_apart_and_never_gates() {
    let saved = baseline(vec![run("a", 4, 1000); 3]);
    let mut slow = vec![run("a", 4, 1000); 3];
    slow[0].wall_ms = 30_000;
    slow[0].transient_ms = Some(26_000);
    slow[1].wall_ms = 9000;
    let work = Work::of(&slow);
    assert_eq!(work.transient_ms, 26_000);
    assert_eq!(work.transient_runs, 1);
    assert_eq!(work.median_wall_ms, Some(9000.0));
    assert_eq!(work.median_wall_less_transient_ms, Some(4000.0));
    assert_eq!(work.model_calls, 12);
    let concluded = conclude(&slow, pin(), pin(), &strict(), Some(&saved));
    assert!(concluded.strict_failures().is_empty());
    let report = concluded.report();
    assert!(
        report.contains("26000 ms lost to transient provider failures over 1 runs"),
        "{report}"
    );
    assert!(
        report.contains("median wall 9000 ms, 4000 ms less transient"),
        "{report}"
    );
}
