//! The native-computer fixture grader, run on recorded events. No Chrome, no network.
use serde_json::{Value, json};
use std::path::PathBuf;

#[path = "../examples/native_computer/support.rs"]
#[allow(dead_code)]
mod support;

const REPORTS: &str = "../../evals/reports/native-computer/2026-09-30";

fn saved_report(relative: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(REPORTS)
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("saved report {} unreadable: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn saved_events(relative: &str) -> Vec<Value> {
    saved_report(relative)["grade"]["events"]
        .as_array()
        .unwrap_or_else(|| panic!("{relative} has no grade.events"))
        .clone()
}

fn trusted(kind: &str, extra: Value) -> Value {
    let mut event = json!({"type": kind, "trusted": true});
    event
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    event
}

fn filters() -> Value {
    trusted("filters", json!({"shift": false}))
}

fn submit(value: &str) -> Value {
    trusted("submit", json!({"value": value}))
}

/// Every event the scripted all-primitives run produces, with one correct submit.
fn primitive_events() -> Vec<Value> {
    vec![
        filters(),
        trusted("input", json!({"value": "orcaA"})),
        submit("orcaA"),
        trusted("double", json!({})),
        trusted("right", json!({})),
        trusted("move", json!({})),
        trusted("filters", json!({"shift": true})),
        trusted("wheel", json!({})),
        trusted("scroll", json!({"top": 200})),
        trusted(
            "drag",
            json!({"path": [[40, 300], [80, 320], [120, 285], [180, 300]]}),
        ),
    ]
}

fn check(grade: &Value, name: &str) -> bool {
    grade["checks"][name]
        .as_bool()
        .unwrap_or_else(|| panic!("no check {name} in {grade}"))
}

#[test]
fn saved_live_report_with_three_wrong_submits_fails_the_strict_grader() {
    let report = saved_report("live-openai/report.json");
    // The recorded verdict came from the old any-submit grader.
    assert_eq!(report["passed"], true);
    let events = saved_events("live-openai/report.json");
    let wrong: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "submit" && e["value"] != "orcaA")
        .map(|e| e["value"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        wrong,
        [
            "penguinorcaA",
            "penguinorcaAorcaA",
            "penguinorcaAorcaAorcaA"
        ]
    );

    let grade = support::grade_events(&events, false);
    assert_eq!(
        grade["passed"], false,
        "three wrong submits then a correct one must not pass"
    );
    assert!(!check(&grade, "submitted_value"));
    assert!(check(&grade, "filters"), "only the submit rule should fail");
    assert_eq!(grade["trusted_submits"], 4);
}

#[test]
fn saved_runs_with_one_correct_submit_still_pass_the_strict_grader() {
    let scripted = support::grade_events(&saved_events("native-protocol.json"), true);
    assert_eq!(scripted["passed"], true, "{scripted}");
    assert_eq!(scripted["trusted_submits"], 1);
    let visible = support::grade_events(&saved_events("visible-chrome/report.json"), false);
    assert_eq!(visible["passed"], true, "{visible}");
    assert_eq!(visible["trusted_submits"], 1);
}

#[test]
fn one_trusted_correct_submit_passes() {
    let grade = support::grade_events(&[filters(), submit("orcaA")], false);
    assert_eq!(grade["passed"], true, "{grade}");
    assert!(check(&grade, "submitted_value"));
    assert_eq!(grade["trusted_submits"], 1);
}

#[test]
fn correct_submit_after_a_wrong_one_fails() {
    let events = [filters(), submit("penguinorcaA"), submit("orcaA")];
    let grade = support::grade_events(&events, false);
    assert_eq!(grade["passed"], false);
    assert!(!check(&grade, "submitted_value"));
    assert_eq!(grade["trusted_submits"], 2);
}

#[test]
fn correct_submit_before_a_wrong_one_fails() {
    let events = [filters(), submit("orcaA"), submit("orcaAorcaA")];
    assert_eq!(support::grade_events(&events, false)["passed"], false);
}

#[test]
fn repeating_the_correct_submit_fails() {
    let events = [filters(), submit("orcaA"), submit("orcaA")];
    let grade = support::grade_events(&events, false);
    assert_eq!(grade["passed"], false);
    assert_eq!(grade["trusted_submits"], 2);
}

#[test]
fn one_wrong_submit_fails() {
    let grade = support::grade_events(&[filters(), submit("orca")], false);
    assert_eq!(grade["passed"], false);
    assert!(!check(&grade, "submitted_value"));
}

#[test]
fn no_submit_fails() {
    let grade = support::grade_events(&[filters()], false);
    assert_eq!(grade["passed"], false);
    assert_eq!(grade["trusted_submits"], 0);
    assert_eq!(support::grade_events(&[], false)["passed"], false);
}

#[test]
fn untrusted_submits_are_not_counted_either_way() {
    let synthetic = |value: &str| json!({"type":"submit","trusted":false,"value":value});
    // A synthetic correct submit proves nothing.
    let only_synthetic = [filters(), synthetic("orcaA")];
    assert_eq!(
        support::grade_events(&only_synthetic, false)["passed"],
        false
    );
    // A missing trust flag is not trusted.
    let unflagged = [filters(), json!({"type":"submit","value":"orcaA"})];
    assert_eq!(support::grade_events(&unflagged, false)["passed"], false);
    // A synthetic extra is not a trusted submit, so the one real submit still counts.
    let extra = [filters(), synthetic("junk"), submit("orcaA")];
    let grade = support::grade_events(&extra, false);
    assert_eq!(grade["passed"], true, "{grade}");
    assert_eq!(grade["trusted_submits"], 1);
}

#[test]
fn other_checks_are_unchanged_without_primitives() {
    // The submit alone is not enough: Show filters must have been clicked trusted.
    let no_filters = support::grade_events(&[submit("orcaA")], false);
    assert_eq!(no_filters["passed"], false);
    assert!(!check(&no_filters, "filters"));
    assert!(check(&no_filters, "submitted_value"));
    let untrusted_filters = json!({"type":"filters","trusted":false,"shift":false});
    let grade = support::grade_events(&[untrusted_filters, submit("orcaA")], false);
    assert_eq!(grade["passed"], false);
    // Without primitives only those two checks exist.
    assert_eq!(grade["checks"].as_object().unwrap().len(), 2);
}

#[test]
fn primitive_mode_passes_a_complete_run_and_keeps_every_check() {
    let events = primitive_events();
    let grade = support::grade_events(&events, true);
    assert_eq!(grade["passed"], true, "{grade}");
    assert_eq!(grade["checks"].as_object().unwrap().len(), 9);
    assert_eq!(grade["events"], json!(events));

    for (kind, check_name) in [
        ("double", "double_click"),
        ("right", "right_click"),
        ("wheel", "wheel_click"),
        ("move", "move"),
        ("scroll", "nested_scroll"),
        ("drag", "drag_path"),
    ] {
        let without: Vec<_> = events
            .iter()
            .filter(|e| e["type"] != kind)
            .cloned()
            .collect();
        let grade = support::grade_events(&without, true);
        assert_eq!(grade["passed"], false, "dropping {kind}");
        assert!(!check(&grade, check_name), "dropping {kind}");
    }
    let no_shift: Vec<_> = events
        .iter()
        .filter(|e| e["shift"] != true)
        .cloned()
        .collect();
    assert!(!check(
        &support::grade_events(&no_shift, true),
        "shift_click"
    ));
    let flat_scroll: Vec<_> = events
        .iter()
        .map(|e| {
            if e["type"] == "scroll" {
                trusted("scroll", json!({"top": 0}))
            } else {
                e.clone()
            }
        })
        .collect();
    assert!(!check(
        &support::grade_events(&flat_scroll, true),
        "nested_scroll"
    ));
    let straight_drag: Vec<_> = events
        .iter()
        .map(|e| {
            if e["type"] == "drag" {
                trusted("drag", json!({"path": [[40, 300], [180, 300]]}))
            } else {
                e.clone()
            }
        })
        .collect();
    assert!(!check(
        &support::grade_events(&straight_drag, true),
        "drag_path"
    ));
}

#[test]
fn primitive_mode_also_requires_exactly_one_submit() {
    let mut events = primitive_events();
    events.push(submit("orcaA"));
    let grade = support::grade_events(&events, true);
    assert_eq!(grade["passed"], false);
    assert!(!check(&grade, "submitted_value"));
    // Every other primitive check still passes.
    for (name, ok) in grade["checks"].as_object().unwrap() {
        assert_eq!(ok == true, name != "submitted_value", "{name}");
    }
}
