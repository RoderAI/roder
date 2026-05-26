use std::path::PathBuf;

use roder_api::inference::RuntimeProfile;

use crate::runner::{OfflineEvalRunnerOptions, run_offline_eval_suite};
use crate::{
    EvalExpectedCommand, EvalExpectedEvidence, EvalExpectedFile, EvalFixture, EvalOutcome,
    EvalWorkspaceSetup,
};

#[tokio::test]
async fn tbench_diagnostic_fixtures_pass_strict_required_output_contracts() {
    let root = std::env::temp_dir().join(format!(
        "roder-evals-tbench-diagnostics-{}",
        uuid::Uuid::new_v4()
    ));
    let fixture_dir = root.join("fixtures");
    let output_dir = root.join("reports");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    for fixture in [
        exact_output_fixture(),
        json_array_fixture(),
        sequence_fixture(),
        numeric_tolerance_fixture(),
        output_directory_hygiene_fixture(),
        visible_verifier_contract_fixture(),
        artifact_checkpoint_fixture(),
        service_target_sanity_fixture(),
        verifier_dependency_parity_fixture(),
    ] {
        std::fs::write(
            fixture_dir.join(format!("{}.json", fixture.id)),
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

    assert_eq!(report.results.len(), 9);
    assert!(
        report
            .results
            .iter()
            .all(|result| result.report.outcome == EvalOutcome::Pass),
        "{:#?}",
        report.results
    );
    let markdown = std::fs::read_to_string(output_dir.join("eval-report.md")).unwrap();
    assert!(markdown.contains("tbench-exact-output-file"));
    assert!(markdown.contains("tbench-json-array-output"));
    assert!(markdown.contains("tbench-sequence-output"));
    assert!(markdown.contains("tbench-numeric-tolerance-output"));
    assert!(markdown.contains("tbench-output-directory-hygiene"));
    assert!(markdown.contains("tbench-visible-verifier-contract"));
    assert!(markdown.contains("tbench-artifact-checkpoint"));
    assert!(markdown.contains("tbench-service-target-sanity"));
    assert!(markdown.contains("tbench-verifier-dependency-parity"));
    for fixture_id in [
        "tbench-numeric-tolerance-output",
        "tbench-output-directory-hygiene",
        "tbench-visible-verifier-contract",
        "tbench-artifact-checkpoint",
        "tbench-service-target-sanity",
        "tbench-verifier-dependency-parity",
    ] {
        let result = report
            .results
            .iter()
            .find(|result| result.fixture_id == fixture_id)
            .unwrap();
        assert_eq!(
            metric_value(result, "verifier_command_checks_required"),
            Some(1.0)
        );
        assert_eq!(
            metric_value(result, "verifier_command_checks_completed"),
            Some(1.0)
        );
    }
    let artifact_checkpoint = report
        .results
        .iter()
        .find(|result| result.fixture_id == "tbench-artifact-checkpoint")
        .unwrap();
    assert_eq!(
        metric_value(artifact_checkpoint, "task_ledger_updates"),
        Some(2.0)
    );
    assert_eq!(
        metric_value(artifact_checkpoint, "task_ledger_completed"),
        Some(2.0)
    );
    let _ = std::fs::remove_dir_all(root);
}

fn metric_value(result: &crate::EvalFixtureResult, name: &str) -> Option<f64> {
    result
        .report
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.value)
}

fn exact_output_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-exact-output-file".to_string(),
        title: "TBench exact output file".to_string(),
        prompt: "FAKE_TBENCH_GCODE_OUTPUT: write the exact required output file.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "required-output".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("out.txt"),
                exists: true,
                contains: Vec::new(),
                exact_contents: Some("flag{gc0d3_iz_ch4LLenGiNg}\n".to_string()),
                max_bytes: None,
                allowed_chars: None,
                json_array_fields: Vec::new(),
            }],
            command_checks: Vec::new(),
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn json_array_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-json-array-output".to_string(),
        title: "TBench JSON array output".to_string(),
        prompt: "FAKE_TBENCH_SAM_JSON: write segmentation coordinates as JSON arrays.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "json-array-output".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("sam-output.json"),
                exists: true,
                contains: Vec::new(),
                exact_contents: None,
                max_bytes: None,
                allowed_chars: None,
                json_array_fields: vec!["coords_x".to_string(), "coords_y".to_string()],
            }],
            command_checks: Vec::new(),
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn sequence_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-sequence-output".to_string(),
        title: "TBench sequence output".to_string(),
        prompt: "FAKE_TBENCH_PROTEIN_SEQUENCE: write a bounded DNA-only sequence file.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "bounded-sequence-output".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("gblock.txt"),
                exists: true,
                contains: Vec::new(),
                exact_contents: None,
                max_bytes: Some(3000),
                allowed_chars: Some("ACGT\n".to_string()),
                json_array_fields: Vec::new(),
            }],
            command_checks: Vec::new(),
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn numeric_tolerance_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-numeric-tolerance-output".to_string(),
        title: "TBench numeric tolerance output".to_string(),
        prompt: "FAKE_TBENCH_VIDEO_FRAME: write a takeoff-frame estimate inside tolerance."
            .to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "numeric-tolerance-output".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("takeoff-frame.txt"),
                exists: true,
                contains: Vec::new(),
                exact_contents: None,
                max_bytes: Some(16),
                allowed_chars: Some("0123456789\n".to_string()),
                json_array_fields: Vec::new(),
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nframe = int(open('takeoff-frame.txt').read())\nassert 219 <= frame <= 223, frame\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn output_directory_hygiene_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-output-directory-hygiene".to_string(),
        title: "TBench output directory hygiene".to_string(),
        prompt: "FAKE_TBENCH_OUTPUT_DIRECTORY_HYGIENE: write only the required submission file."
            .to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "output-directory-hygiene".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("submission/main.rs"),
                exists: true,
                contains: vec!["fn main()".to_string()],
                exact_contents: None,
                max_bytes: None,
                allowed_chars: None,
                json_array_fields: Vec::new(),
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nfrom pathlib import Path\nentries = sorted(p.name for p in Path('submission').iterdir())\nassert entries == ['main.rs'], entries\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn visible_verifier_contract_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-visible-verifier-contract".to_string(),
        title: "TBench visible verifier contract".to_string(),
        prompt: "FAKE_TBENCH_VISIBLE_VERIFIER_CONTRACT: read visible verifier constants and write the exact required result.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "visible-verifier-contract".to_string(),
        ],
        workspace: EvalWorkspaceSetup {
            files: vec![crate::EvalWorkspaceFile {
                path: PathBuf::from("tests/test_contract.py"),
                contents: "EXPECTED = 'GritLM/GritLM-7B'\n".to_string(),
            }],
            commands: Vec::new(),
        },
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("result.txt"),
                exists: true,
                contains: Vec::new(),
                exact_contents: Some("GritLM/GritLM-7B\n".to_string()),
                max_bytes: Some(64),
                allowed_chars: None,
                json_array_fields: Vec::new(),
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nfrom pathlib import Path\nexpected = 'GritLM/GritLM-7B'\nassert Path('tests/test_contract.py').read_text().count(expected) == 1\nassert Path('result.txt').read_text() == expected + '\\n'\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn artifact_checkpoint_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-artifact-checkpoint".to_string(),
        title: "TBench artifact checkpoint".to_string(),
        prompt: "FAKE_TBENCH_ARTIFACT_CHECKPOINT: create the required scoreable artifact before finalizing and complete the task ledger.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "artifact-checkpoint".to_string(),
            "task-ledger".to_string(),
        ],
        workspace: EvalWorkspaceSetup::default(),
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("primers.fasta"),
                exists: true,
                contains: vec![">primer_forward".to_string(), "ACGTACGTACGT".to_string()],
                exact_contents: None,
                max_bytes: Some(256),
                allowed_chars: Some(">_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\n".to_string()),
                json_array_fields: Vec::new(),
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nfrom pathlib import Path\ntext = Path('primers.fasta').read_text()\nassert text.startswith('>primer_forward\\n'), text\nassert '>primer_reverse\\n' in text, text\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: true,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn service_target_sanity_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-service-target-sanity".to_string(),
        title: "TBench service target sanity".to_string(),
        prompt: "FAKE_TBENCH_SERVICE_TARGET_SANITY: write evidence that validation targeted the guest service endpoint, not the host.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "service-target".to_string(),
            "environment-target".to_string(),
        ],
        workspace: EvalWorkspaceSetup {
            files: vec![
                crate::EvalWorkspaceFile {
                    path: PathBuf::from("host-uname.txt"),
                    contents: "Darwin host-kernel\n".to_string(),
                },
                crate::EvalWorkspaceFile {
                    path: PathBuf::from("guest-uname.txt"),
                    contents: "Linux alpine-vm 6.6.0\n".to_string(),
                },
            ],
            commands: Vec::new(),
        },
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("target-check.json"),
                exists: true,
                contains: vec![
                    "\"target\":\"guest\"".to_string(),
                    "\"guestMarker\":\"alpine-vm\"".to_string(),
                ],
                exact_contents: None,
                max_bytes: Some(256),
                allowed_chars: None,
                json_array_fields: Vec::new(),
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nimport json\nfrom pathlib import Path\npayload = json.loads(Path('target-check.json').read_text())\nassert payload['target'] == 'guest', payload\nassert payload['guestMarker'] == 'alpine-vm', payload\nassert payload['sshPort'] == 2222, payload\nassert 'Darwin' not in json.dumps(payload), payload\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}

fn verifier_dependency_parity_fixture() -> EvalFixture {
    EvalFixture {
        id: "tbench-verifier-dependency-parity".to_string(),
        title: "TBench verifier dependency parity".to_string(),
        prompt: "FAKE_TBENCH_VERIFIER_DEPENDENCY_PARITY: read the visible verifier dependency contract and write the fallback verifier command plus assertion names.".to_string(),
        tags: vec![
            "tbench-diagnostic".to_string(),
            "verifier-dependency-parity".to_string(),
            "environment-target".to_string(),
        ],
        workspace: EvalWorkspaceSetup {
            files: vec![crate::EvalWorkspaceFile {
                path: PathBuf::from("tests/verifier_requirements.py"),
                contents: "REQUIRED_ASSERTIONS = ['coords_array_type', 'row_parallel_shape']\nFALLBACK_COMMAND = 'python3 -m pytest tests/test_outputs.py'\n".to_string(),
            }],
            commands: Vec::new(),
        },
        timeout_ms: Some(10_000),
        expected: EvalExpectedEvidence {
            final_answer_contains: vec!["hello from roder".to_string()],
            files: vec![EvalExpectedFile {
                path: PathBuf::from("verifier-parity.json"),
                exists: true,
                contains: vec![
                    "\"source\":\"visible-verifier\"".to_string(),
                    "\"coords_array_type\"".to_string(),
                    "\"row_parallel_shape\"".to_string(),
                ],
                exact_contents: None,
                max_bytes: Some(512),
                allowed_chars: None,
                json_array_fields: vec!["assertions".to_string()],
            }],
            command_checks: vec![EvalExpectedCommand {
                command: "python3 - <<'PY'\nimport ast, json\nfrom pathlib import Path\npayload = json.loads(Path('verifier-parity.json').read_text())\nrequirements = ast.literal_eval(Path('tests/verifier_requirements.py').read_text().split('=', 1)[1].split('\\n', 1)[0].strip())\nassert payload['source'] == 'visible-verifier', payload\nassert payload['fallbackCommand'] == 'python3 -m pytest tests/test_outputs.py', payload\nassert payload['assertions'] == requirements, payload\nPY".to_string(),
                expected_exit_code: 0,
                stdout_contains: Vec::new(),
                stderr_contains: Vec::new(),
            }],
            verification_required: true,
            task_ledger_required: false,
        },
        constraints: Vec::new(),
        lazy_discovery: None,
    }
}
