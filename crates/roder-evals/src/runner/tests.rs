use std::path::PathBuf;

use crate::runner::{
    EvalProfileMode, EvalSpeedPolicyMode, OfflineEvalRunnerOptions, run_offline_eval_suite,
};
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
            task_ledger_required: false,
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
            task_ledger_required: false,
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
async fn task_ledger_required_fixtures_grade_missing_and_complete_state() {
    let root = std::env::temp_dir().join(format!("roder-evals-test-{}", uuid::Uuid::new_v4()));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    for (id, prompt) in [
        (
            "task-ledger-complete",
            "FAKE_TASK_LEDGER_COMPLETE: complete a decomposed task.",
        ),
        ("task-ledger-missing", "complete a decomposed task."),
    ] {
        let fixture = EvalFixture {
            id: id.to_string(),
            title: id.to_string(),
            prompt: prompt.to_string(),
            tags: vec!["task-ledger".to_string()],
            workspace: EvalWorkspaceSetup::default(),
            timeout_ms: Some(10_000),
            expected: EvalExpectedEvidence {
                final_answer_contains: vec!["hello from roder".to_string()],
                files: Vec::new(),
                command_checks: Vec::new(),
                verification_required: false,
                task_ledger_required: true,
            },
            constraints: Vec::new(),
        };
        std::fs::write(
            fixture_dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();
    }

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

    let complete = report
        .results
        .iter()
        .find(|result| result.fixture_id == "task-ledger-complete")
        .unwrap();
    assert_eq!(complete.report.outcome, EvalOutcome::Pass);
    assert!(
        complete
            .report
            .trajectory
            .events
            .iter()
            .any(|event| event.event_type == "task_ledger_updated")
    );

    let missing = report
        .results
        .iter()
        .find(|result| result.fixture_id == "task-ledger-missing")
        .unwrap();
    assert_eq!(missing.report.outcome, EvalOutcome::Fail);
    assert_eq!(
        missing.report.failure_class,
        Some(EvalFailureClass::Verifier)
    );
    assert!(
        missing
            .failure_message
            .as_deref()
            .unwrap_or_default()
            .contains("task ledger was required")
    );
    let markdown = std::fs::read_to_string(output_dir.join("eval-report.md")).unwrap();
    assert!(markdown.contains("Task Ledger Metrics"));
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
            task_ledger_required: false,
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

#[tokio::test]
async fn speed_policy_both_runs_baseline_and_speed_report_rows() {
    let root = std::env::temp_dir().join(format!("roder-evals-test-{}", uuid::Uuid::new_v4()));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = EvalFixture {
        id: "routine-speed".to_string(),
        title: "Routine speed".to_string(),
        prompt: "Say hello after inspecting the routine task.".to_string(),
        tags: vec!["speed".to_string()],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: Vec::new(),
            command_checks: Vec::new(),
            verification_required: false,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
    };
    std::fs::write(
        fixture_dir.join("routine-speed.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();

    let report = run_offline_eval_suite(
        &fixture_dir,
        OfflineEvalRunnerOptions {
            output_dir: output_dir.clone(),
            speed_policy: EvalSpeedPolicyMode::Both,
            ..OfflineEvalRunnerOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(report.results.len(), 2);
    assert!(report.results.iter().any(|result| {
        result
            .report
            .run
            .tags
            .iter()
            .any(|tag| tag == "speed_policy:off")
    }));
    assert!(report.results.iter().any(|result| {
        result
            .report
            .run
            .tags
            .iter()
            .any(|tag| tag == "speed_policy:on")
    }));
    let speed = report
        .results
        .iter()
        .find(|result| {
            result
                .report
                .run
                .tags
                .iter()
                .any(|tag| tag == "speed_policy:on")
        })
        .unwrap();
    assert!(
        speed
            .report
            .metrics
            .iter()
            .any(|metric| { metric.name == "deadline_remaining_seconds" && metric.value > 0.0 })
    );
    let markdown = std::fs::read_to_string(output_dir.join("eval-report.md")).unwrap();
    assert!(markdown.contains("Speed Metrics"));
    assert!(markdown.contains("Speed Policy Comparison"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn model_profile_all_runs_profile_delta_report_rows() {
    let root = std::env::temp_dir().join(format!(
        "roder-evals-model-profile-{}",
        uuid::Uuid::new_v4()
    ));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = EvalFixture {
        id: "profile-edit-tool".to_string(),
        title: "Profile edit tool".to_string(),
        prompt: "Say hello from the selected profile.".to_string(),
        tags: vec!["model-profile".to_string(), "edit-tool".to_string()],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: Vec::new(),
            command_checks: Vec::new(),
            verification_required: false,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
    };
    std::fs::write(
        fixture_dir.join("profile-edit-tool.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();

    let report = run_offline_eval_suite(
        &fixture_dir,
        OfflineEvalRunnerOptions {
            output_dir: output_dir.clone(),
            profiles: EvalProfileMode::All,
            ..OfflineEvalRunnerOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(report.results.len(), 2);
    assert!(report.results.iter().any(|result| {
        result
            .report
            .run
            .tags
            .iter()
            .any(|tag| tag == "profile:gpt-5.5")
    }));
    assert!(report.results.iter().any(|result| {
        result
            .report
            .run
            .tags
            .iter()
            .any(|tag| tag == "profile:claude-haiku-4-5-20251001")
    }));
    let markdown = std::fs::read_to_string(output_dir.join("eval-report.md")).unwrap();
    assert!(markdown.contains("Model Profile Deltas"));
    assert!(markdown.contains("profile:gpt-5.5") || markdown.contains("gpt-5.5"));
    let _ = std::fs::remove_dir_all(root);
}
