#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
GENERATE = ROOT / "evals/harbor/generate_tbench_campaign.py"
VALIDATE = ROOT / "evals/harbor/validate_tbench_campaign.py"


class ValidateTbenchCampaignTests(unittest.TestCase):
    def test_generated_campaign_manifest_validates(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))

            result = validate_campaign(manifest)

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("TBench campaign validation passed", result.stdout)

    def test_rejects_route_config_task_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            route = json.loads(manifest.read_text())["routes"][0]
            config_path = Path(route["config"])
            config = json.loads(config_path.read_text())
            config["datasets"][0]["task_names"] = ["wrong-task"]
            config["datasets"][0]["n_tasks"] = 1
            config_path.write_text(json.dumps(config, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("route medium-validated task_names mismatch", result.stderr)

    def test_rejects_route_analysis_path_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            data = json.loads(manifest.read_text())
            data["routes"][0]["analysisJson"] = "/tmp/wrong-analysis.json"
            manifest.write_text(json.dumps(data, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("route medium-validated analysisJson mismatch", result.stderr)

    def test_rejects_missing_route_image_manifest_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            data = json.loads(manifest.read_text())
            data["routes"][0].pop("imageManifest", None)
            manifest.write_text(json.dumps(data, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("route medium-validated imageManifest is missing", result.stderr)

    def test_rejects_missing_run_script(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            data = json.loads(manifest.read_text())
            Path(data["runScript"]).unlink()

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript cannot be executed", result.stderr)

    def test_rejects_non_executable_run_script(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            data = json.loads(manifest.read_text())
            path = Path(data["runScript"])
            path.chmod(path.stat().st_mode & ~(stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript cannot be executed", result.stderr)

    def test_requires_matching_route_image_preflight_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertEqual(0, result.returncode, result.stderr)

    def test_uses_explicit_route_image_preflight_manifest_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            first = data["routes"][0]
            explicit_manifest = Path(temp) / "custom-image-preflight.json"
            original_manifest = first["imageManifest"]
            first["imageManifest"] = str(explicit_manifest)
            manifest.write_text(json.dumps(data, indent=2) + "\n")
            run_script = Path(data["runScript"])
            run_script.write_text(
                run_script.read_text().replace(original_manifest, str(explicit_manifest))
            )
            write_image_manifest(
                explicit_manifest,
                config=first["config"],
                tasks=first["taskCount"],
                task_names=first["tasks"],
            )
            for route in data["routes"][1:]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertEqual(0, result.returncode, result.stderr)

    def test_rejects_preflight_manifest_for_different_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )
            first = data["routes"][0]
            write_image_manifest(
                output_dir / f"{first['name']}-images.json",
                config="evals/harbor/tbench-full-gpt55-medium.json",
                tasks=first["taskCount"],
                task_names=first["tasks"],
            )

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("image preflight config mismatch", result.stderr)

    def test_rejects_route_image_preflight_task_mapping_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )
            first = data["routes"][0]
            manifest_path = output_dir / f"{first['name']}-images.json"
            image_manifest = json.loads(manifest_path.read_text())
            image_manifest["images"][0]["tasks"] = [first["tasks"][1]]
            image_manifest["images"][1]["tasks"] = [first["tasks"][0]]
            manifest_path.write_text(json.dumps(image_manifest, indent=2) + "\n")

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated imagePreflight manifest image task mapping mismatch",
            result.stderr,
        )

    def test_rejects_route_image_preflight_partial_present_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )
            first = data["routes"][0]
            manifest_path = output_dir / f"{first['name']}-images.json"
            image_manifest = json.loads(manifest_path.read_text())
            image_manifest["summary"]["present"] = first["taskCount"] - 1
            image_manifest["tasks"][-1]["status"] = "unknown"
            manifest_path.write_text(json.dumps(image_manifest, indent=2) + "\n")

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated imagePreflight present does not cover all tasks",
            result.stderr,
        )

    def test_rejects_route_image_preflight_task_name_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_image_manifest(
                    output_dir / f"{route['name']}-images.json",
                    config=route["config"],
                    tasks=route["taskCount"],
                    task_names=route["tasks"],
                )
            first = data["routes"][0]
            manifest_path = output_dir / f"{first['name']}-images.json"
            image_manifest = json.loads(manifest_path.read_text())
            image_manifest["tasks"][0]["task_name"] = "stale-task"
            image_manifest["images"][0]["tasks"] = ["stale-task"]
            manifest_path.write_text(json.dumps(image_manifest, indent=2) + "\n")

            result = validate_campaign(
                manifest,
                "--require-image-preflight",
                "--preflight-dir",
                str(output_dir),
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated imagePreflight manifest task names mismatch",
            result.stderr,
        )

    def test_requires_clean_route_analysis_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)

            result = validate_campaign(manifest, "--require-analysis")

        self.assertEqual(0, result.returncode, result.stderr)

    def test_rejects_missing_route_analysis_output(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("route medium-validated analysis JSON cannot be read", result.stderr)

    def test_rejects_unclean_route_analysis_output(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)
            write_analysis_outputs(data["routes"][0], clean=False)

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("route medium-validated analysis is not clean", result.stderr)

    def test_rejects_route_analysis_baseline_blockers(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)
            first = data["routes"][0]
            analysis_path = Path(first["analysisJson"])
            analysis = json.loads(analysis_path.read_text())
            analysis["classes"]["provider_stream_incomplete"] = [
                {"task_name": first["tasks"][0]}
            ]
            analysis_path.write_text(json.dumps(analysis, indent=2) + "\n")

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated analysis baseline blocked: provider_stream_incomplete",
            result.stderr,
        )

    def test_rejects_route_analysis_task_name_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)
            first = data["routes"][0]
            analysis_path = Path(first["analysisJson"])
            analysis = json.loads(analysis_path.read_text())
            analysis["classes"]["pass"] = [
                {"task_name": f"stale-task-{index}"}
                for index in range(first["taskCount"])
            ]
            analysis_path.write_text(json.dumps(analysis, indent=2) + "\n")

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated analysis task names mismatch",
            result.stderr,
        )

    def test_rejects_route_analysis_duplicate_scored_task_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)
            first = data["routes"][0]
            analysis_path = Path(first["analysisJson"])
            analysis = json.loads(analysis_path.read_text())
            analysis["classes"]["pass"].append({"task_name": first["tasks"][0]})
            analysis_path.write_text(json.dumps(analysis, indent=2) + "\n")

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated analysis scored task entries mismatch",
            result.stderr,
        )

    def test_rejects_route_analysis_with_zero_harbor_trials(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            for route in data["routes"]:
                write_analysis_outputs(route, clean=True)
            first = data["routes"][0]
            analysis_path = Path(first["analysisJson"])
            analysis = json.loads(analysis_path.read_text())
            analysis["stats"]["harbor"]["n_trials"] = 0
            analysis_path.write_text(json.dumps(analysis, indent=2) + "\n")

            result = validate_campaign(manifest, "--require-analysis")

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route medium-validated analysis trial count mismatch",
            result.stderr,
        )

    def test_rejects_route_config_without_prebuilt_binary_injection(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            route = json.loads(manifest.read_text())["routes"][0]
            config_path = Path(route["config"])
            config = json.loads(config_path.read_text())
            config["agents"][0]["kwargs"]["include_prebuilt_binary"] = "false"
            config_path.write_text(json.dumps(config, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("include_prebuilt_binary", result.stderr)

    def test_rejects_route_config_with_local_source_upload_enabled(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            route = json.loads(manifest.read_text())["routes"][0]
            config_path = Path(route["config"])
            config = json.loads(config_path.read_text())
            config["agents"][0]["kwargs"]["include_local_source"] = "true"
            config_path.write_text(json.dumps(config, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("include_local_source", result.stderr)

    def test_rejects_route_config_deadline_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            route = json.loads(manifest.read_text())["routes"][0]
            config_path = Path(route["config"])
            config = json.loads(config_path.read_text())
            config["agents"][0]["override_timeout_sec"] = 900
            config["agents"][0]["kwargs"]["soft_timeout_sec"] = 890
            config["agents"][0]["kwargs"]["speed_policy_eval_deadline_seconds"] = 870
            config_path.write_text(json.dumps(config, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        text = result.stderr
        self.assertIn("override_timeout_sec", text)
        self.assertIn("soft_timeout_sec", text)
        self.assertIn("speed_policy_eval_deadline_seconds", text)

    def test_rejects_route_config_hash_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp))
            route = json.loads(manifest.read_text())["routes"][0]
            config_path = Path(route["config"])
            config = json.loads(config_path.read_text())
            config["debug_note"] = "changed after manifest generation"
            config_path.write_text(json.dumps(config, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("configSha256 mismatch", result.stderr)


def generate_campaign(output_dir: Path) -> Path:
    result = subprocess.run(
        [
            "python3",
            str(GENERATE),
            "--output-dir",
            str(output_dir),
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return output_dir / "validated-conversions-manifest.json"


def validate_campaign(manifest: Path, *extra_args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(VALIDATE), str(manifest), *extra_args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def write_image_manifest(
    path: Path,
    *,
    config: str,
    tasks: int,
    task_names: list[str] | None = None,
) -> None:
    names = task_names or [f"task-{index}" for index in range(tasks)]
    manifest = {
        "config": config,
        "clean": True,
        "summary": {
            "tasks": tasks,
            "unique_images": tasks,
            "present": tasks,
            "missing": 0,
            "unresolved": 0,
            "pull_failed": 0,
        },
        "selection_errors": [],
        "tasks": [
            {
                "task_name": names[index],
                "status": "present",
                "image": f"image-{index}",
            }
            for index in range(tasks)
        ],
        "images": [
            {"image": f"image-{index}", "tasks": [names[index]]}
            for index in range(tasks)
        ],
    }
    path.write_text(json.dumps(manifest, indent=2) + "\n")


def write_analysis_outputs(route: dict, *, clean: bool) -> None:
    task_count = int(route["taskCount"])
    analysis = {
        "clean": clean,
        "stats": {
            "harbor": {"n_errors": 0, "n_trials": task_count},
            "task_dirs": task_count,
            "passes": task_count if clean else 0,
            "scored_failures": 0 if clean else task_count,
            "harness_error_classes": {} if clean else {"unknown_error": 1},
        },
        "classes": {"pass": [{"task_name": task} for task in route["tasks"]]}
        if clean
        else {"unknown_error": [{"task_name": route["tasks"][0]}]},
    }
    Path(route["analysisJson"]).write_text(json.dumps(analysis, indent=2) + "\n")
    Path(route["analysisMarkdown"]).write_text("# route analysis\n")


if __name__ == "__main__":
    unittest.main()
