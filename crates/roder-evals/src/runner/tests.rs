use std::path::PathBuf;

use crate::runner::{OfflineEvalRunnerOptions, run_offline_eval_suite};
use crate::{
    EvalExpectedCommand, EvalExpectedEvidence, EvalExpectedFile, EvalFailureClass, EvalFixture,
    EvalOutcome, EvalWorkspaceFile, EvalWorkspaceSetup,
};
use roder_api::inference::RuntimeProfile;

#[tokio::test]
async fn runner_creates_report_files_from_fake_provider_fixture() {
    let root = std::env::temp_dir().join(format!("roder-evals-test-{}", uuid::Uuid::new_v4()));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = EvalFixture {
        id: "hello".to_string(),
        title: "Hello".to_string(),
        prompt: "Say hello.".to_string(),
        tags: vec!["fake-provider".to_string()],
        workspace: EvalWorkspaceSetup {
            files: vec![EvalWorkspaceFile {
                path: PathBuf::from("README.md"),
                contents: "ready\n".to_string(),
            }],
            commands: Vec::new(),
        },
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("README.md"),
                exists: true,
                contains: vec!["ready".to_string()],
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "test -f README.md && printf checked".to_string(),
                expected_exit_code: 0,
                stdout_contains: vec!["checked".to_string()],
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
        },
        constraints: Vec::new(),
    };
    std::fs::write(
        fixture_dir.join("hello.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();

    let report = run_offline_eval_suite(
        &fixture_dir,
        OfflineEvalRunnerOptions {
            output_dir: output_dir.clone(),
            ..OfflineEvalRunnerOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].report.outcome, EvalOutcome::Pass);
    assert!(output_dir.join("eval-run.json").exists());
    assert!(output_dir.join("eval-report.md").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn eval_runtime_profile_fails_clarification_waits_without_hanging() {
    let root = std::env::temp_dir().join(format!("roder-evals-test-{}", uuid::Uuid::new_v4()));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = EvalFixture {
        id: "clarification-wait".to_string(),
        title: "Clarification wait".to_string(),
        prompt: "FAKE_REQUEST_USER_INPUT: ask for clarification first.".to_string(),
        tags: vec!["runtime-profile".to_string()],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: Vec::new(),
            command_checks: Vec::new(),
            verification_required: false,
        },
        constraints: Vec::new(),
    };
    std::fs::write(
        fixture_dir.join("clarification-wait.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();

    let report = run_offline_eval_suite(
        &fixture_dir,
        OfflineEvalRunnerOptions {
            output_dir: output_dir.clone(),
            runtime_profile: RuntimeProfile::Eval,
            ..OfflineEvalRunnerOptions::default()
        },
    )
    .await
    .unwrap();

    let result = &report.results[0];
    assert_eq!(result.report.outcome, EvalOutcome::Fail);
    assert_eq!(result.report.failure_class, Some(EvalFailureClass::Runtime));
    assert!(
        result
            .failure_message
            .as_deref()
            .unwrap_or_default()
            .contains("clarification unavailable")
    );
    assert_eq!(
        result.report.trajectory.events[0]
            .runtime_profile
            .as_deref(),
        Some("eval")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn failed_fixture_reports_failure_class_and_trace_excerpt() {
    let root = std::env::temp_dir().join(format!("roder-evals-test-{}", uuid::Uuid::new_v4()));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = EvalFixture {
        id: "tool-misuse-fail".to_string(),
        title: "Tool misuse fail".to_string(),
        prompt: "Use valid tool arguments.".to_string(),
        tags: vec!["tool-misuse".to_string()],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["used valid tool arguments".to_string()],
            files: Vec::new(),
            command_checks: Vec::new(),
            verification_required: false,
        },
        constraints: Vec::new(),
    };
    std::fs::write(
        fixture_dir.join("tool-misuse-fail.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();

    let report = run_offline_eval_suite(
        &fixture_dir,
        OfflineEvalRunnerOptions {
            output_dir: output_dir.clone(),
            ..OfflineEvalRunnerOptions::default()
        },
    )
    .await
    .unwrap();

    let result = &report.results[0];
    assert_eq!(result.report.outcome, EvalOutcome::Fail);
    assert_eq!(
        result.report.failure_class,
        Some(EvalFailureClass::ToolSchema)
    );
    assert!(!result.trace_excerpt.is_empty());
    let markdown = std::fs::read_to_string(output_dir.join("eval-report.md")).unwrap();
    assert!(markdown.contains("ToolSchema"));
    assert!(markdown.contains("inference_event"));
    let _ = std::fs::remove_dir_all(root);
}
