//! Offline edit-surface eval matrix (roadmap phase 80, Task 6).
//!
//! Loads every fixture under `evals/fixtures/edit-tools/` (including the
//! `matrix/` classes) and runs it through the real `roder-edit-core` APIs —
//! the same code backing `roder-tools` and `@roderai/edit-tools` — then
//! aggregates a regression metric report: first-attempt success, wrong-edit
//! rate, fuzzy accept/refuse counts, patch success/failure, no-op rate, and
//! refusal diagnostics quality.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roder_edit_core::{
    EditApplyError, EditMatchMode, EditOptions, ReadFormatOptions, TextEdit,
    apply_codex_patch_to_workspace, apply_multi_edit, format_line_numbered_read,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FixtureFile {
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    initial: Option<String>,
    #[serde(default)]
    files: Option<BTreeMap<String, String>>,
    #[serde(default)]
    generated_lines: Option<GeneratedLines>,
    operation: Operation,
    #[serde(default)]
    expected: Option<String>,
    #[serde(default)]
    expected_files: Option<BTreeMap<String, String>>,
    #[serde(default)]
    absent_files: Vec<String>,
    #[serde(default)]
    expect_error: Option<String>,
    #[serde(default)]
    min_candidates: usize,
    #[serde(default)]
    read_contains: Vec<String>,
    #[serde(default)]
    read_absent: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GeneratedLines {
    count: usize,
    prefix: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Operation {
    Edit {
        old_string: String,
        new_string: String,
        #[serde(default)]
        fuzzy: Option<String>,
        #[serde(default)]
        reindent: bool,
    },
    MultiEdit {
        edits: Vec<TextEdit>,
        #[serde(default)]
        fuzzy: Option<String>,
        #[serde(default)]
        reindent: bool,
    },
    Patch {
        patch: String,
    },
    Read {
        start_line: usize,
        limit: usize,
    },
}

#[derive(Debug, Default)]
struct Metrics {
    total: usize,
    first_attempt_success: usize,
    wrong_edits: usize,
    no_ops: usize,
    fuzzy_accepts: usize,
    fuzzy_refusals: usize,
    exact_refusals: usize,
    patch_successes: usize,
    patch_failures: usize,
    reads: usize,
    refusals_without_candidates: usize,
    failures: Vec<String>,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/fixtures/edit-tools")
}

fn load_fixtures(dir: &Path, fixtures: &mut Vec<Fixture>) {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("fixture dir")
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            load_fixtures(&path, fixtures);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let file: FixtureFile = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        fixtures.extend(file.fixtures);
    }
}

fn fixture_text(fixture: &Fixture) -> String {
    if let Some(initial) = &fixture.initial {
        return initial.clone();
    }
    if let Some(generated) = &fixture.generated_lines {
        return (1..=generated.count)
            .map(|index| format!("{} {index}", generated.prefix))
            .collect::<Vec<_>>()
            .join("\n");
    }
    fixture
        .files
        .as_ref()
        .and_then(|files| files.get("fixture.txt"))
        .cloned()
        .unwrap_or_default()
}

fn edit_options(fuzzy: &Option<String>, reindent: bool) -> EditOptions {
    EditOptions {
        fuzzy: match fuzzy.as_deref() {
            Some("off") => EditMatchMode::Off,
            Some("apply_safe") => EditMatchMode::ApplySafe,
            _ => EditMatchMode::Diagnose,
        },
        reindent_inserted: reindent,
        ..EditOptions::default()
    }
}

fn error_kind(error: &EditApplyError) -> (&'static str, usize) {
    match error {
        EditApplyError::OldStringNotFound { candidates, .. } => {
            ("old_string_not_found", candidates.len())
        }
        EditApplyError::OldStringAmbiguous { candidates, .. } => {
            ("old_string_ambiguous", candidates.len())
        }
    }
}

fn run_fixture(fixture: &Fixture, metrics: &mut Metrics) {
    metrics.total += 1;
    let fail = |message: String, metrics: &mut Metrics| {
        metrics
            .failures
            .push(format!("{}: {message}", fixture.name));
    };

    match &fixture.operation {
        Operation::Edit { .. } | Operation::MultiEdit { .. } => {
            let (edits, options) = match &fixture.operation {
                Operation::Edit {
                    old_string,
                    new_string,
                    fuzzy,
                    reindent,
                } => (
                    vec![TextEdit {
                        old_string: old_string.clone(),
                        new_string: new_string.clone(),
                    }],
                    edit_options(fuzzy, *reindent),
                ),
                Operation::MultiEdit {
                    edits,
                    fuzzy,
                    reindent,
                } => (edits.clone(), edit_options(fuzzy, *reindent)),
                _ => unreachable!(),
            };
            let text = fixture_text(fixture);
            let uses_fuzzy = options.fuzzy == EditMatchMode::ApplySafe;
            match apply_multi_edit("fixture.txt", &text, &edits, options) {
                Ok((updated, _outcome)) => {
                    if let Some(expected_error) = &fixture.expect_error {
                        metrics.wrong_edits += 1;
                        fail(
                            format!("expected error {expected_error} but edit applied"),
                            metrics,
                        );
                        return;
                    }
                    let expected = fixture.expected.as_deref().unwrap_or_default();
                    if updated == expected {
                        metrics.first_attempt_success += 1;
                        if updated == text {
                            metrics.no_ops += 1;
                        }
                        if uses_fuzzy && !text.contains(&edits[0].old_string) {
                            metrics.fuzzy_accepts += 1;
                        }
                    } else {
                        metrics.wrong_edits += 1;
                        fail(
                            format!(
                                "result mismatch:\n--- got ---\n{updated}\n--- want ---\n{expected}"
                            ),
                            metrics,
                        );
                    }
                }
                Err(error) => {
                    let (kind, candidates) = error_kind(&error);
                    if uses_fuzzy {
                        metrics.fuzzy_refusals += 1;
                    } else {
                        metrics.exact_refusals += 1;
                    }
                    if candidates == 0 {
                        metrics.refusals_without_candidates += 1;
                    }
                    match &fixture.expect_error {
                        Some(expected) if expected == kind => {
                            metrics.first_attempt_success += 1;
                            if candidates < fixture.min_candidates {
                                fail(
                                    format!(
                                        "refusal returned {candidates} candidates, expected at least {}",
                                        fixture.min_candidates
                                    ),
                                    metrics,
                                );
                            }
                        }
                        Some(expected) => {
                            fail(format!("expected error {expected}, got {kind}"), metrics)
                        }
                        None => fail(format!("unexpected error {kind}"), metrics),
                    }
                }
            }
        }
        Operation::Patch { patch } => {
            let root = std::env::temp_dir().join(format!(
                "roder-edit-eval-{}-{}",
                fixture.name,
                uuid_like()
            ));
            std::fs::create_dir_all(&root).unwrap();
            if let Some(files) = &fixture.files {
                for (rel, contents) in files {
                    let path = root.join(rel);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, contents).unwrap();
                }
            } else {
                std::fs::write(root.join("fixture.txt"), fixture_text(fixture)).unwrap();
            }
            match apply_codex_patch_to_workspace(&root, patch) {
                Ok(_) => {
                    if fixture.expect_error.is_some() {
                        metrics.patch_failures += 1;
                        fail(
                            "expected patch failure but patch applied".to_string(),
                            metrics,
                        );
                    } else {
                        metrics.patch_successes += 1;
                        let mut ok = true;
                        if let Some(expected) = &fixture.expected {
                            let got = std::fs::read_to_string(root.join("fixture.txt"))
                                .unwrap_or_default();
                            if &got != expected {
                                ok = false;
                                fail(format!("patched fixture.txt mismatch: {got:?}"), metrics);
                            }
                        }
                        if let Some(expected_files) = &fixture.expected_files {
                            for (rel, expected) in expected_files {
                                let got =
                                    std::fs::read_to_string(root.join(rel)).unwrap_or_default();
                                if &got != expected {
                                    ok = false;
                                    fail(format!("patched {rel} mismatch: {got:?}"), metrics);
                                }
                            }
                        }
                        for rel in &fixture.absent_files {
                            if root.join(rel).exists() {
                                ok = false;
                                fail(format!("{rel} should have been removed"), metrics);
                            }
                        }
                        if ok {
                            metrics.first_attempt_success += 1;
                        } else {
                            metrics.wrong_edits += 1;
                        }
                    }
                }
                Err(error) => {
                    metrics.patch_failures += 1;
                    if fixture.expect_error.as_deref() == Some("apply_patch_failed") {
                        metrics.first_attempt_success += 1;
                    } else {
                        fail(format!("unexpected patch failure: {error}"), metrics);
                    }
                }
            }
            let _ = std::fs::remove_dir_all(root);
        }
        Operation::Read { start_line, limit } => {
            metrics.reads += 1;
            let text = fixture_text(fixture);
            let page = format_line_numbered_read(
                &text,
                ReadFormatOptions {
                    start_line: *start_line,
                    limit: *limit,
                },
            );
            let mut ok = true;
            for needle in &fixture.read_contains {
                if !page.contains(needle) {
                    ok = false;
                    fail(format!("read output missing {needle:?}"), metrics);
                }
            }
            for needle in &fixture.read_absent {
                if page.contains(needle) {
                    ok = false;
                    fail(format!("read output must not contain {needle:?}"), metrics);
                }
            }
            let shown = page.lines().count();
            if shown > *limit {
                ok = false;
                fail(
                    format!("read returned {shown} lines, limit was {limit}"),
                    metrics,
                );
            }
            if ok {
                metrics.first_attempt_success += 1;
            }
        }
    }
    let _ = &fixture.class;
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[test]
fn edit_surface_eval_matrix_passes_with_zero_wrong_edits() {
    let mut fixtures = Vec::new();
    load_fixtures(&fixture_dir(), &mut fixtures);
    assert!(
        fixtures.len() >= 15,
        "expected the committed fixture matrix, found {}",
        fixtures.len()
    );

    let mut metrics = Metrics::default();
    for fixture in &fixtures {
        run_fixture(fixture, &mut metrics);
    }

    eprintln!(
        "edit-surface eval metrics: total={} first_attempt_success={} wrong_edits={} no_ops={} \
         fuzzy_accepts={} fuzzy_refusals={} exact_refusals={} patch_successes={} patch_failures={} \
         reads={} refusals_without_candidates={}",
        metrics.total,
        metrics.first_attempt_success,
        metrics.wrong_edits,
        metrics.no_ops,
        metrics.fuzzy_accepts,
        metrics.fuzzy_refusals,
        metrics.exact_refusals,
        metrics.patch_successes,
        metrics.patch_failures,
        metrics.reads,
        metrics.refusals_without_candidates,
    );

    assert!(
        metrics.failures.is_empty(),
        "fixture failures:\n{}",
        metrics.failures.join("\n")
    );
    assert_eq!(metrics.wrong_edits, 0, "wrong-edit rate must stay zero");
    assert_eq!(
        metrics.first_attempt_success, metrics.total,
        "every fixture must succeed on the first attempt"
    );
    assert!(
        metrics.fuzzy_accepts >= 1,
        "matrix must include a successful fuzzy recovery case"
    );
    assert!(
        metrics.fuzzy_refusals >= 1,
        "matrix must include a refused fuzzy case"
    );
    assert!(
        metrics.patch_successes >= 1 && metrics.patch_failures >= 1,
        "matrix must include patch success and fail-closed cases"
    );
    assert_eq!(
        metrics.refusals_without_candidates, 0,
        "every refusal must include candidate diagnostics"
    );
}
