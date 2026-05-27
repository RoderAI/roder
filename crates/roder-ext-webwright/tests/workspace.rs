use std::path::{Path, PathBuf};
use std::process::Command;

use roder_ext_webwright::{
    ReportResult, ReportSection, WebwrightReport, WebwrightTaskDefinition, WebwrightWorkspace,
    export_workspace, sanitize_task_id, verify_workspace,
};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/fixtures/webwright")
        .join(path)
}

#[test]
fn fixture_workspace_summarizes_required_artifacts() {
    let summary = WebwrightWorkspace::new(fixture("basic_success"))
        .summary()
        .unwrap();

    assert_eq!(summary.latest_run, Some(1));
    assert_eq!(summary.plan.checked_count, 3);
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
        summary.validation_errors.is_empty(),
        "{:?}",
        summary.validation_errors
    );
    assert_eq!(
        summary
            .task_definition
            .as_ref()
            .and_then(|task| task.short_id.as_deref()),
        Some("basic_success")
    );
    assert_eq!(
        summary
            .report
            .as_ref()
            .and_then(|report| report.result.headline.as_deref()),
        Some("Fixture result")
    );
}

#[test]
fn fixture_workspace_rejects_contract_gaps() {
    assert!(
        WebwrightWorkspace::new(fixture("missing_log"))
            .validate()
            .is_err()
    );
    assert!(
        WebwrightWorkspace::new(fixture("full_page_marker"))
            .validate()
            .is_err()
    );
}

#[test]
fn offline_eval_fixtures_cover_required_webwright_cases() {
    let one_shot = WebwrightWorkspace::new(fixture("basic_success"))
        .summary()
        .unwrap();
    assert_eq!(
        one_shot
            .manifest
            .as_ref()
            .map(|manifest| manifest.mode.as_str()),
        Some("run")
    );
    assert!(one_shot.report.is_some());

    let craft = WebwrightWorkspace::new(fixture("craft_success"))
        .summary()
        .unwrap();
    assert_eq!(
        craft
            .manifest
            .as_ref()
            .map(|manifest| manifest.mode.as_str()),
        Some("craft")
    );
    assert!(craft.final_script.import_safe);
    let craft_script = std::fs::read_to_string(fixture("craft_success/final_script.py")).unwrap();
    assert!(craft_script.contains("argparse"));
    let craft_temp = std::env::temp_dir().join(format!(
        "roder-webwright-craft-fixture-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&craft_temp).unwrap();
    std::fs::copy(
        fixture("craft_success/final_script.py"),
        craft_temp.join("final_script.py"),
    )
    .unwrap();
    let no_args = Command::new("python3")
        .arg("final_script.py")
        .current_dir(&craft_temp)
        .output()
        .unwrap();
    assert!(
        no_args.status.success(),
        "craft fixture no-arg run failed: {}",
        String::from_utf8_lossy(&no_args.stderr)
    );
    let help = Command::new("python3")
        .arg("final_script.py")
        .arg("--help")
        .current_dir(&craft_temp)
        .output()
        .unwrap();
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("--heading"));
    assert!(help_text.contains("Fixture Heading"));
    assert!(verify_workspace(fixture("craft_success")).passed);

    assert!(!verify_workspace(fixture("missing_log")).passed);
    assert!(!verify_workspace(fixture("blocked_site")).passed);

    let dependency_error =
        std::fs::read_to_string(fixture("missing_browser_binary/dependency_error.json")).unwrap();
    assert!(dependency_error.contains("playwright install firefox"));
}

#[test]
fn task_id_sanitization_is_stable() {
    assert_eq!(
        sanitize_task_id("Find $4.99 Movies on CheapCharts!"),
        "find-4-99-movies-on-cheapcharts"
    );
}

#[test]
fn workspace_writers_roundtrip_structured_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "roder-webwright-writers-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = WebwrightWorkspace::new(&root);
    std::fs::create_dir_all(root.join("final_runs/run_001/screenshots")).unwrap();
    std::fs::write(
        root.join("final_runs/run_001/final_script.py"),
        "if __name__ == \"__main__\":\n    pass\n",
    )
    .unwrap();
    std::fs::write(
        root.join("final_runs/run_001/final_script_log.txt"),
        "final datum: ok\n",
    )
    .unwrap();
    std::fs::write(
        root.join("final_runs/run_001/screenshots/final_execution_001_ok.png"),
        "fake",
    )
    .unwrap();

    workspace.write_plan("- [x] CP1: done\n").unwrap();
    workspace
        .write_final_script("if __name__ == \"__main__\":\n    pass\n")
        .unwrap();
    workspace
        .write_task_definition(&WebwrightTaskDefinition {
            task_id: "roundtrip".to_string(),
            short_id: Some("roundtrip".to_string()),
            title: None,
            theme: None,
            cadence: None,
            level: None,
            website: None,
            task_prompt: "Roundtrip writer".to_string(),
            num_steps: Some(1),
        })
        .unwrap();
    workspace
        .write_report(&WebwrightReport {
            sources: Vec::new(),
            result: ReportResult {
                headline: Some("Roundtrip".to_string()),
                sections: vec![ReportSection {
                    section_type: "summary".to_string(),
                    title: "Final".to_string(),
                    body: Some("ok".to_string()),
                    columns: Vec::new(),
                    rows: Vec::new(),
                    entries: Vec::new(),
                }],
            },
        })
        .unwrap();
    workspace
        .write_self_reflect_result(1, &serde_json::json!({ "predicted_label": "success" }))
        .unwrap();

    let summary = workspace.summary().unwrap();
    assert_eq!(summary.plan.checked_count, 1);
    assert!(summary.final_script.import_safe);
    assert_eq!(
        summary
            .task_definition
            .as_ref()
            .map(|task| task.task_id.as_str()),
        Some("roundtrip")
    );
    assert_eq!(
        summary
            .report
            .as_ref()
            .and_then(|report| report.result.headline.as_deref()),
        Some("Roundtrip")
    );
    assert_eq!(
        summary.runs[0]
            .self_reflect
            .as_ref()
            .and_then(|reflect| reflect.predicted_label.as_deref()),
        Some("success")
    );
}

#[test]
fn export_workspace_copies_only_sanitized_shareable_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "roder-webwright-export-source-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let export = std::env::temp_dir().join(format!(
        "roder-webwright-export-target-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(root.join("final_runs/run_001/screenshots")).unwrap();
    std::fs::write(root.join("webwright.json"), "{}").unwrap();
    std::fs::write(root.join("plan.md"), "- [x] CP1: done\n").unwrap();
    std::fs::write(root.join("final_script.py"), "print('ok')\n").unwrap();
    std::fs::write(root.join("task.json"), "{\"task_id\":\"export\"}").unwrap();
    std::fs::write(root.join("report.json"), "{\"result\":{\"sections\":[]}}").unwrap();
    std::fs::write(root.join("cookies.json"), "{\"token\":\"secret\"}").unwrap();
    std::fs::write(
        root.join("final_runs/run_001/final_script.py"),
        "print('ok')\n",
    )
    .unwrap();
    std::fs::write(
        root.join("final_runs/run_001/final_script_log.txt"),
        "Authorization: Bearer secret\nfinal datum: ok\n",
    )
    .unwrap();
    std::fs::write(
        root.join("final_runs/run_001/screenshots/final_execution_001_ok.png"),
        "fake",
    )
    .unwrap();
    std::fs::write(
        root.join("final_runs/run_001/screenshots/browser_state.json"),
        "{}",
    )
    .unwrap();

    let result = export_workspace(&WebwrightWorkspace::new(&root), &export).unwrap();

    assert!(result.files.contains(&"webwright-export.json".to_string()));
    assert!(
        result
            .files
            .contains(&"final_runs/run_001/screenshots/final_execution_001_ok.png".to_string())
    );
    assert!(result.excluded.contains(&"cookies.json".to_string()));
    assert!(
        result
            .excluded
            .contains(&"final_runs/run_001/screenshots/browser_state.json".to_string())
    );
    let exported_log =
        std::fs::read_to_string(export.join("final_runs/run_001/final_script_log.txt")).unwrap();
    assert!(exported_log.contains("[redacted sensitive Webwright output line]"));
    assert!(exported_log.contains("final datum: ok"));
}
