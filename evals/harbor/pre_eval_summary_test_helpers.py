"""Shared helpers for pre-eval summary tests."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

from tbench_diagnostic_test_data import (
    default_metrics_for_fixture,
    diagnostic_fixture_ids,
)


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"
MISSING_VALIDATION = "__missing_validation__"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def write_eval_run(
    directory: Path,
    outcomes: list[str],
    *,
    metrics: list[list[dict]] | None = None,
) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    fixture_ids = diagnostic_fixture_ids()
    effective_outcomes = list(outcomes)
    if effective_outcomes and len(effective_outcomes) < len(fixture_ids):
        effective_outcomes.extend(
            "pass" for _ in range(len(fixture_ids) - len(effective_outcomes))
        )
    result_metrics = []
    for index, _outcome in enumerate(effective_outcomes):
        fixture_id = fixture_ids[index] if index < len(fixture_ids) else f"fixture-{index}"
        if metrics is not None and index < len(metrics):
            result_metrics.append(metrics[index])
        else:
            result_metrics.append(default_metrics_for_fixture(fixture_id))
    (directory / "eval-run.json").write_text(
        json.dumps(
            {
                "results": [
                    {
                        "fixtureId": (
                            fixture_ids[index]
                            if index < len(fixture_ids)
                            else f"fixture-{index}"
                        ),
                        "report": {
                            "outcome": outcome,
                            "metrics": result_metrics[index],
                        },
                    }
                    for index, outcome in enumerate(effective_outcomes)
                ]
            }
        )
    )


def write_validation(directory: Path, status: str) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    if status != MISSING_VALIDATION:
        (directory / "validation.json").write_text(json.dumps({"status": status}))


def write_image_manifest(
    path: Path,
    *,
    clean: bool,
    tasks: int = 1,
    present: int = 1,
    missing: int = 0,
) -> None:
    path.write_text(
        json.dumps(
            {
                "clean": clean,
                "summary": {
                    "tasks": tasks,
                    "unique_images": tasks,
                    "present": present,
                    "missing": missing,
                    "unresolved": 0,
                    "pull_failed": 0,
                },
            }
        )
    )


def build_summary(
    module,
    root: Path,
    *,
    tbench_outcomes: list[str] | None = None,
    tbench_metrics: list[list[dict]] | None = None,
    speed_outcomes: list[str] | None = None,
    speed_dir: Path | None = None,
    analysis_status: str | None = None,
    image_manifest: Path | None = None,
    run_tests: bool = True,
    include_speed: bool = False,
    require_prebuilt: bool = False,
    preflight_images: bool = False,
    pull_images: bool = False,
    image_config: str = "",
    require_auth: bool = False,
    prebuilt_binary: Path | None = None,
    auth_file: Path | None = None,
    harbor_readiness_status: str = "passed",
    harbor_harness_tests_status: str | None = None,
    roder_evals_status: str | None = None,
    failure_step: str = "",
    failure_exit_code: int | None = None,
) -> dict:
    tbench_dir = root / "tbench-diagnostics"
    tbench_dir.mkdir(exist_ok=True)
    if tbench_outcomes is not None:
        write_eval_run(tbench_dir, tbench_outcomes, metrics=tbench_metrics)
    if speed_outcomes is not None:
        speed_dir = root / "speed-policy"
        write_eval_run(speed_dir, speed_outcomes)

    analysis_dir = None
    analysis_target = ""
    if analysis_status is not None:
        analysis_dir = root / "harbor-analysis-baseline"
        write_validation(analysis_dir, analysis_status)
        analysis_target = "analysis.json"

    return module.build_summary(
        output_root=root,
        tbench_dir=tbench_dir,
        speed_dir=speed_dir,
        analysis_dir=analysis_dir,
        run_tests=run_tests,
        include_speed=include_speed,
        require_prebuilt=require_prebuilt,
        preflight_images=preflight_images,
        pull_images=pull_images,
        image_config=image_config,
        analysis_target=analysis_target,
        analysis_baseline="baseline.json",
        prebuilt_binary=prebuilt_binary or root / "missing-roder",
        auth_file=auth_file or root / "codex.json",
        require_auth=require_auth,
        image_manifest=image_manifest,
        harbor_readiness_status=harbor_readiness_status,
        harbor_harness_tests_status=harbor_harness_tests_status,
        roder_evals_status=roder_evals_status,
        failure_step=failure_step,
        failure_exit_code=failure_exit_code,
    )
