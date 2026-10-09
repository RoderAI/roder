//! The tally, the baseline and what a run concludes from them, offline.

use super::grade::Marks;
use super::pin::Pin;
use super::tally::{Baseline, Run, Settings, baseline_path, conclude, plan, repeats, tally};

fn pin() -> Pin {
    Pin {
        commit: "c0ffee".into(),
        worktree: "aaaa".into(),
        corpus: "bbbb".into(),
        model: "jev-latest".into(),
        setup: "task values".into(),
    }
}

fn run(task: &str, verdict_ok: bool, truth_ok: bool, steps: usize) -> Run<'_> {
    Run {
        task,
        marks: Marks::new(verdict_ok, truth_ok),
        steps,
        input_tokens: Some(1000 * steps as u64),
    }
}

/// `n` runs, of which `pass` pass and the rest fail honestly: the
/// verdict misses too, so none of them is a false green.
fn runs(task: &str, n: usize, pass: usize) -> Vec<Run<'_>> {
    (0..n).map(|i| run(task, i < pass, i < pass, 3)).collect()
}

/// `n` runs that claimed success over a page that did not bear it out.
fn false_greens(task: &str, n: usize) -> Vec<Run<'_>> {
    (0..n).map(|_| run(task, true, false, 3)).collect()
}

fn settings(n: usize) -> Settings {
    Settings {
        n,
        subset: false,
        save: true,
    }
}

#[test]
fn repeats_default_to_one_and_reject_anything_that_is_not_a_count() {
    assert_eq!(repeats(None).unwrap(), 1);
    assert_eq!(repeats(Some("  ")).unwrap(), 1);
    assert_eq!(repeats(Some("3")).unwrap(), 3);
    assert_eq!(repeats(Some(" 20 ")).unwrap(), 20);
    for bad in ["0", "21", "-1", "three", "2.5"] {
        let error = repeats(Some(bad)).unwrap_err().to_string();
        assert!(error.contains("JEV_EVAL_N"), "{bad}: {error}");
    }
}

#[test]
fn a_tally_counts_each_task_over_its_runs() {
    let mut all = vec![
        run("a", true, true, 2),
        run("a", true, false, 4),
        run("a", false, false, 9),
    ];
    all.extend(runs("b", 2, 2));
    let tally = tally(all);
    let a = &tally["a"];
    assert_eq!(
        (a.runs, a.passed, a.verdict_ok, a.truth_ok, a.false_green),
        (3, 1, 2, 1, 1)
    );
    assert_eq!(a.median_steps, 4.0);
    assert_eq!(a.median_input_tokens, Some(4000.0));
    assert_eq!(tally["b"].passed, 2);
    assert_eq!(tally["b"].false_green, 0);
}

#[test]
fn a_median_of_an_even_count_is_the_mean_of_the_middle_two() {
    let tally = tally([run("a", true, true, 2), run("a", true, true, 5)]);
    assert_eq!(tally["a"].median_steps, 3.5);
}

#[test]
fn runs_that_reported_no_tokens_do_not_pull_the_median_down() {
    let mut quiet = run("a", true, true, 3);
    quiet.input_tokens = None;
    let counted = tally([quiet, run("a", true, true, 3), run("a", true, true, 5)]);
    assert_eq!(counted["a"].median_input_tokens, Some(4000.0));
    let none = {
        let mut one = run("a", true, true, 3);
        one.input_tokens = None;
        tally([one])
    };
    assert_eq!(none["a"].median_input_tokens, None);
}

fn baseline(task: &str, n: usize, pass: usize) -> Baseline {
    Baseline {
        pin: pin(),
        n,
        tasks: tally(runs(task, n, pass)),
    }
}

#[test]
fn a_baseline_round_trips_through_its_file() {
    let dir = std::env::temp_dir().join(format!("jev-baseline-{}", std::process::id()));
    let path = dir.join("nested/live-baseline.json");
    assert_eq!(Baseline::load(&path).unwrap(), None);
    let saved = baseline("a", 3, 2);
    saved.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("}\n"), "{text}");
    assert!(text.contains("\"worktree\": \"aaaa\""), "{text}");
    assert_eq!(Baseline::load(&path).unwrap(), Some(saved));

    // A hand edit that adds a field is an error, never a baseline that
    // quietly ignores it.
    std::fs::write(&path, text.replace("\"n\": 3", "\"n\": 3, \"extra\": 1")).unwrap();
    assert!(Baseline::load(&path).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_lower_rate_is_a_regression_and_runs_of_unequal_count_compare() {
    let saved = baseline("a", 3, 3);
    // 3 of 3 became 3 of 5.
    let now = tally(runs("a", 5, 3));
    let comparison = saved.compare(&now, &pin());
    assert_eq!(
        comparison.regressions,
        ["a: passed 3/3 -> 3/5, verdict_ok 3/3 -> 3/5, truth_ok 3/3 -> 3/5"]
    );
    // The same rate on more runs is not worse.
    let now = tally(runs("a", 6, 6));
    assert!(saved.compare(&now, &pin()).regressions.is_empty());
}

#[test]
fn a_new_false_green_is_named_among_the_ways_a_task_got_worse() {
    let saved = baseline("a", 3, 3);
    let mut again = runs("a", 2, 2);
    again.extend(false_greens("a", 1));
    let comparison = saved.compare(&tally(again), &pin());
    assert_eq!(comparison.regressions.len(), 1);
    assert!(
        comparison.regressions[0].contains("false_green 0/3 -> 1/3"),
        "{comparison:?}"
    );
}

#[test]
fn a_known_failure_that_stays_failing_is_not_a_regression() {
    let saved = baseline("icon_by_picture", 3, 0);
    let comparison = saved.compare(&tally(runs("icon_by_picture", 3, 0)), &pin());
    assert!(comparison.regressions.is_empty(), "{comparison:?}");
    assert!(comparison.improved.is_empty());
}

#[test]
fn a_better_rate_is_named_and_a_task_without_a_baseline_is_listed() {
    let saved = baseline("a", 3, 1);
    let mut now = tally(runs("a", 3, 3));
    now.extend(tally(runs("fresh", 3, 3)));
    let comparison = saved.compare(&now, &pin());
    assert_eq!(comparison.improved, ["a: passed 1/3 -> 3/3"]);
    assert_eq!(comparison.new, ["fresh"]);
    assert!(comparison.regressions.is_empty());
}

#[test]
fn a_different_corpus_model_or_setup_is_noted() {
    let saved = baseline("a", 3, 3);
    let other = Pin {
        corpus: "cccc".into(),
        model: "jev-1.14.0".into(),
        setup: "text model".into(),
        // The code is meant to differ: that is what is being measured.
        commit: "deadbeef".into(),
        worktree: "dddd".into(),
    };
    let comparison = saved.compare(&tally(runs("a", 3, 3)), &other);
    assert_eq!(comparison.notes.len(), 3, "{comparison:?}");
    assert!(comparison.notes[0].starts_with("corpus differs"));
}

#[test]
fn a_run_that_kept_its_pin_may_become_the_baseline() {
    let all = runs("a", 3, 3);
    let conclusion = conclude(&all, pin(), pin(), &settings(3), None);
    assert!(conclusion.drift.is_empty());
    assert!(conclusion.refused.is_empty(), "{:?}", conclusion.refused);
    let saved = conclusion.save.expect("a baseline to save");
    assert_eq!(saved.pin, pin());
    assert_eq!(saved.n, 3);
    assert_eq!(saved.tasks["a"].passed, 3);
}

#[test]
fn a_mid_run_edit_is_flagged_and_no_baseline_is_saved_from_it() {
    let edited = Pin {
        worktree: "eeee".into(),
        ..pin()
    };
    let conclusion = conclude(&runs("a", 3, 3), pin(), edited, &settings(3), None);
    assert_eq!(conclusion.drift, ["worktree aaaa -> eeee"]);
    assert!(conclusion.save.is_none());
    assert_eq!(conclusion.refused.len(), 1);
    assert!(conclusion.refused[0].contains("changed during the run"));
    let report = conclusion.report();
    assert!(
        report.contains("MID-RUN EDIT: worktree aaaa -> eeee"),
        "{report}"
    );
    assert!(report.contains("baseline not saved"), "{report}");
    // And strict mode fails on it, though every run passed.
    assert_eq!(conclusion.strict_failures().len(), 1);
}

#[test]
fn too_few_runs_and_a_narrowed_corpus_are_not_baselines() {
    let few = conclude(&runs("a", 2, 2), pin(), pin(), &settings(2), None);
    assert!(few.save.is_none());
    assert!(few.refused[0].contains("at least 3"), "{:?}", few.refused);

    let narrow = Settings {
        subset: true,
        ..settings(3)
    };
    let narrowed = conclude(&runs("a", 3, 3), pin(), pin(), &narrow, None);
    assert!(narrowed.save.is_none());
    assert!(narrowed.refused[0].contains("JEV_EVAL_TASKS"));

    // Not asking to save is not a refusal.
    let quiet = Settings {
        save: false,
        ..settings(1)
    };
    let conclusion = conclude(&runs("a", 1, 1), pin(), pin(), &quiet, None);
    assert!(conclusion.save.is_none() && conclusion.refused.is_empty());
}

#[test]
fn strict_without_a_baseline_wants_every_run_to_pass() {
    let no_save = Settings {
        save: false,
        ..settings(3)
    };
    let all_pass = conclude(&runs("a", 3, 3), pin(), pin(), &no_save, None);
    assert!(all_pass.strict_failures().is_empty());
    let one_fails = conclude(&runs("a", 3, 2), pin(), pin(), &no_save, None);
    assert_eq!(one_fails.strict_failures(), ["1 failed runs"]);
}

#[test]
fn strict_against_a_baseline_fails_on_a_task_getting_worse_not_on_known_failures() {
    let no_save = Settings {
        save: false,
        ..settings(3)
    };
    let saved = baseline("a", 3, 3);
    // A task that failed 3 of 3 in the baseline and still does passes.
    let mut known = baseline("b", 3, 0);
    known.tasks.extend(saved.tasks.clone());
    let mut now = runs("a", 3, 3);
    now.extend(runs("b", 3, 0));
    let held = conclude(&now, pin(), pin(), &no_save, Some(&known));
    assert!(
        held.strict_failures().is_empty(),
        "{:?}",
        held.strict_failures()
    );

    let mut worse = runs("a", 3, 1);
    worse.extend(runs("b", 3, 0));
    let slipped = conclude(&worse, pin(), pin(), &no_save, Some(&known));
    let failures = slipped.strict_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].starts_with("worse than the baseline: a:"));
    assert!(slipped.report().contains("REGRESSION vs baseline: a:"));
}

#[test]
fn a_false_green_fails_strict_even_against_a_baseline() {
    let no_save = Settings {
        save: false,
        ..settings(3)
    };
    // The baseline already had a false green in every run, so the rate
    // is not worse; strict still names them.
    let known = Baseline {
        pin: pin(),
        n: 3,
        tasks: tally(false_greens("a", 3)),
    };
    let now = false_greens("a", 3);
    let conclusion = conclude(&now, pin(), pin(), &no_save, Some(&known));
    assert!(
        conclusion
            .comparison
            .as_ref()
            .unwrap()
            .regressions
            .is_empty()
    );
    assert_eq!(conclusion.strict_failures(), ["3 false green runs"]);
}

#[test]
fn the_summary_serialises_the_pins_and_leaves_the_baseline_out() {
    let conclusion = conclude(&runs("a", 3, 3), pin(), pin(), &settings(3), None);
    let value = serde_json::to_value(&conclusion).unwrap();
    assert_eq!(value["start"]["commit"], "c0ffee");
    assert_eq!(value["end"]["corpus"], "bbbb");
    assert_eq!(value["drift"], serde_json::json!([]));
    assert_eq!(value["tasks"]["a"]["runs"], 3);
    assert!(value.get("save").is_none() && value.get("refused").is_none());
}

#[test]
fn every_task_runs_once_before_any_runs_twice() {
    let order = plan(&["a", "b"], 2)
        .into_iter()
        .map(|(task, repeat)| format!("{task}{repeat}"))
        .collect::<Vec<_>>();
    assert_eq!(order, ["a1", "b1", "a2", "b2"]);
    assert_eq!(plan(&["a", "b"], 1).len(), 2);
}

#[test]
fn the_baseline_is_a_file_in_the_corpus_unless_one_is_named() {
    let default = baseline_path(None);
    assert!(
        default.ends_with("tests/fixtures/evals/live-baseline.json"),
        "{default:?}"
    );
    assert_eq!(baseline_path(Some(" ")), default);
    assert_eq!(
        baseline_path(Some("/tmp/mine.json")),
        std::path::PathBuf::from("/tmp/mine.json")
    );
}

#[test]
fn a_run_with_no_rows_does_not_hold_under_strict() {
    let quiet = Settings {
        save: false,
        ..settings(1)
    };
    let nothing = conclude(&[], pin(), pin(), &quiet, None);
    assert_eq!(nothing.strict_failures(), ["no run finished"]);
}
