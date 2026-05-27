use std::path::{Path, PathBuf};

use crate::workspace::{WebwrightWorkspace, scoped_path};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/fixtures/webwright")
        .join(path)
}

#[test]
fn loads_and_validates_basic_fixture() {
    let workspace = WebwrightWorkspace::new(fixture("basic_success"));
    let summary = workspace.summary().unwrap();

    assert_eq!(summary.latest_run, Some(1));
    assert!(
        summary.validation_errors.is_empty(),
        "{:?}",
        summary.validation_errors
    );
    assert_eq!(summary.plan.total_count, 3);
    assert_eq!(summary.plan.checked_count, 3);
    assert!(summary.final_script.import_safe);
    assert_eq!(summary.runs[0].screenshots.len(), 1);
    assert!(summary.runs[0].final_script.import_safe);
    assert_eq!(
        summary.runs[0].log.final_datum.as_deref(),
        Some("final datum: Fixture Heading")
    );
    assert_eq!(
        summary.runs[0]
            .self_reflect
            .as_ref()
            .and_then(|reflect| reflect.predicted_label.as_deref()),
        Some("success")
    );
    assert!(
        summary.runs[0]
            .log_tail
            .iter()
            .any(|line| line.contains("final datum"))
    );
}

#[test]
fn rejects_missing_latest_run_log() {
    let workspace = WebwrightWorkspace::new(fixture("missing_log"));
    let err = workspace.validate().unwrap_err().to_string();
    assert!(err.contains("missing final_script_log.txt"), "{err}");
}

#[test]
fn rejects_full_page_screenshot_marker() {
    let workspace = WebwrightWorkspace::new(fixture("full_page_marker"));
    let err = workspace.validate().unwrap_err().to_string();
    assert!(
        err.contains("full-page screenshots are not allowed"),
        "{err}"
    );
}

#[test]
fn prevents_paths_escaping_workspace() {
    let workspace = WebwrightWorkspace::new(fixture("basic_success"));
    assert!(workspace.resolve_inside("final_script.py").is_ok());
    assert!(workspace.resolve_inside("../secret.txt").is_err());
    assert!(workspace.resolve_inside("/tmp/secret.txt").is_err());
}

#[test]
fn scoped_path_rejects_parent_component_escapes() {
    let root = fixture("basic_success");

    assert!(scoped_path(&root, ".roder/webwright/task", "outputDir").is_ok());
    assert!(scoped_path(&root, "../secret", "outputDir").is_err());
    assert!(scoped_path(&root, root.join("../secret"), "outputDir").is_err());
}
