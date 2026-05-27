#!/usr/bin/env python3

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
VALIDATE = HARBOR_DIR / "validate_tbench_campaign.py"
WRITE_PLAN = HARBOR_DIR / "write_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import (  # noqa: E402
    auth_json,
    clean_summary,
    executable_file,
    linux_x86_64_elf_bytes,
)
from tbench_campaign_test_helpers import GENERATE, generate_campaign  # noqa: E402


class ValidateTbenchCampaignLaunchPlanTests(unittest.TestCase):
    def test_require_launch_plans_rejects_missing_route_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")

            result = validate_campaign_with_launch_plans(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-validated launchPlan cannot be read",
            result.stderr,
        )

    def test_require_launch_plans_accepts_written_dry_run_route_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_campaign(temp_path / "campaign")
            route = generated_route(manifest)
            write_route_image_manifest(route)
            summary = write_route_pre_eval_summary(temp_path, route)
            write_route_launch_plan(route, summary, dry_run=True)

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertEqual(0, result.returncode, result.stderr)

    def test_require_launch_plans_rejects_stale_route_plan_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_campaign(temp_path / "campaign")
            route = generated_route(manifest)
            write_route_image_manifest(route)
            summary = write_route_pre_eval_summary(temp_path, route)
            write_route_launch_plan(route, summary, dry_run=True)
            plan_path = Path(route["launchPlan"])
            plan = json.loads(plan_path.read_text())
            plan["harborConfig"] = "evals/harbor/tbench-smoke.json"
            plan_path.write_text(json.dumps(plan, indent=2) + "\n")

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-validated launchPlan harborConfig mismatch",
            result.stderr,
        )

    def test_require_launch_plans_rejects_route_manifest_source_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_campaign(temp_path / "campaign")
            route = generated_route(manifest)
            write_route_image_manifest(route)
            summary = write_route_pre_eval_summary(temp_path, route)
            write_route_launch_plan(route, summary, dry_run=True)
            mutate_route_plan(route, "imagePreflightSource", "pre_eval_summary")

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-validated launchPlan imagePreflightSource mismatch",
            result.stderr,
        )

    def test_require_launch_plans_rejects_mixed_pre_eval_summaries(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_default_campaign(temp_path / "campaign")
            routes = generated_routes(manifest)
            for route in routes:
                write_route_image_manifest(route)
            summary = write_campaign_pre_eval_summary(temp_path, routes)
            for route in routes:
                write_route_launch_plan(route, summary, dry_run=True)
            plan_path = Path(routes[-1]["launchPlan"])
            plan = json.loads(plan_path.read_text())
            plan["preEvalSummarySha256"] = "f" * 64
            plan_path.write_text(json.dumps(plan, indent=2) + "\n")

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-plan-first launchPlan preEvalSummarySha256 mismatch",
            result.stderr,
        )

    def test_require_launch_plans_rejects_mixed_launch_plan_options(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_default_campaign(temp_path / "campaign")
            routes = generated_routes(manifest)
            for route in routes:
                write_route_image_manifest(route)
            summary = write_campaign_pre_eval_summary(temp_path, routes)
            for route in routes:
                write_route_launch_plan(route, summary, dry_run=True)

            for field, value in (
                ("preEvalOutputDir", str(temp_path / "other-pre-eval")),
                ("preEvalRanHere", True),
                ("requireCampaignSummary", True),
                ("requireAnalysis", True),
                ("pullPreflight", True),
                ("offlinePreflight", False),
            ):
                with self.subTest(field=field):
                    mutate_route_plan(routes[-1], field, value)
                    try:
                        result = validate_campaign_with_launch_plans(
                            manifest,
                            allow_dry_run=True,
                        )

                        self.assertNotEqual(0, result.returncode)
                        self.assertIn(
                            f"route xhigh-plan-first launchPlan {field} mismatch",
                            result.stderr,
                        )
                    finally:
                        mutate_route_plan(
                            routes[-1],
                            field,
                            original_plan_value(routes[0], field),
                        )

    def test_require_launch_plans_rejects_mixed_execution_modes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_default_campaign(temp_path / "campaign")
            routes = generated_routes(manifest)
            for route in routes:
                write_route_image_manifest(route)
            summary = write_campaign_pre_eval_summary(temp_path, routes)
            for route in routes:
                write_route_launch_plan(route, summary, dry_run=True)
            plan_path = Path(routes[-1]["launchPlan"])
            plan = json.loads(plan_path.read_text())
            plan["launchStatus"] = "ready"
            plan["dryRun"] = False
            plan["wouldRunHarbor"] = True
            plan_path.write_text(json.dumps(plan, indent=2) + "\n")

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-plan-first launchPlan launchStatus mismatch",
            result.stderr,
        )

    def test_require_launch_plans_rejects_mixed_campaign_summary_bindings(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_default_campaign(temp_path / "campaign")
            routes = generated_routes(manifest)
            for route in routes:
                write_route_image_manifest(route)
            summary = write_campaign_pre_eval_summary(temp_path, routes)
            add_campaign_summary_to_pre_eval_summary(summary)
            for route in routes:
                write_route_launch_plan(route, summary, dry_run=True)
            mutate_route_plan_nested(
                routes[-1],
                ("campaignSummary", "summaryJsonSha256"),
                "f" * 64,
            )

            result = validate_campaign_with_launch_plans(
                manifest,
                allow_dry_run=True,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "route xhigh-plan-first launchPlan campaignSummary.summaryJsonSha256 mismatch",
            result.stderr,
        )


def generated_route(manifest: Path) -> dict:
    route = generated_routes(manifest)[0]
    assert route["name"] == "xhigh-validated"
    return route


def generated_routes(manifest: Path) -> list[dict]:
    data = json.loads(manifest.read_text())
    return data["routes"]


def generate_default_campaign(output_dir: Path) -> Path:
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


def validate_campaign_with_launch_plans(
    manifest: Path,
    *,
    allow_dry_run: bool = True,
) -> subprocess.CompletedProcess[str]:
    command = [
        "python3",
        str(VALIDATE),
        str(manifest),
        "--require-launch-plans",
    ]
    if allow_dry_run:
        command.append("--allow-dry-run-launch-plans")
    return subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def write_route_pre_eval_summary(root: Path, route: dict) -> Path:
    return write_campaign_pre_eval_summary(root, [route])


def write_campaign_pre_eval_summary(root: Path, routes: list[dict]) -> Path:
    prebuilt = executable_file(root / "roder-linux-amd64", linux_x86_64_elf_bytes())
    auth = root / "codex.json"
    auth.write_text(auth_json())
    primary_config = Path(routes[0]["config"])
    summary_data = clean_summary(
        prebuilt=prebuilt,
        auth=auth,
        config=primary_config,
        image_preflight=False,
    )
    summary_data["outputDir"] = str(root / "pre-eval")
    summary_data["checks"]["harborConfigs"]["entries"] = [
        {
            "path": str(Path(route["config"])),
            "sha256": sha256(Path(route["config"]).read_bytes()).hexdigest(),
        }
        for route in routes
    ]
    summary = root / "pre-eval-summary.json"
    summary.write_text(json.dumps(summary_data) + "\n")
    return summary


def write_route_image_manifest(route: dict) -> None:
    tasks = [str(task) for task in route["tasks"]]
    task_rows = [
        {
            "task_name": task,
            "image": f"terminalbench/{task}:latest",
            "status": "present",
            "image_source": "task",
        }
        for task in tasks
    ]
    image_rows = [
        {
            "image": row["image"],
            "tasks": [row["task_name"]],
        }
        for row in task_rows
    ]
    manifest = {
        "clean": True,
        "config": route["config"],
        "offline": True,
        "pull": False,
        "summary": {
            "tasks": len(tasks),
            "unique_images": len(tasks),
            "present": len(tasks),
            "missing": 0,
            "unresolved": 0,
            "pull_failed": 0,
        },
        "selection_errors": [],
        "tasks": task_rows,
        "images": image_rows,
    }
    Path(route["imageManifest"]).write_text(json.dumps(manifest, indent=2) + "\n")


def write_route_launch_plan(route: dict, summary: Path, *, dry_run: bool) -> None:
    command = [
        "python3",
        str(WRITE_PLAN),
        "--output",
        route["launchPlan"],
        "--pre-eval-summary",
        str(summary),
        "--pre-eval-output-dir",
        str(summary.parent / "pre-eval"),
        "--job-dir",
        route["jobDir"],
        "--harbor-config",
        route["config"],
        "--analysis-json",
        route["analysisJson"],
        "--analysis-markdown",
        route["analysisMarkdown"],
        "--max-pre-eval-age-seconds",
        "7200",
        "--image-preflight-manifest",
        route["imageManifest"],
        "--require-image-preflight",
    ]
    if dry_run:
        command.append("--dry-run")
    result = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)


def mutate_route_plan(route: dict, field: str, value: object) -> None:
    plan_path = Path(route["launchPlan"])
    plan = json.loads(plan_path.read_text())
    plan[field] = value
    plan_path.write_text(json.dumps(plan, indent=2) + "\n")


def original_plan_value(route: dict, field: str) -> object:
    return json.loads(Path(route["launchPlan"]).read_text())[field]


def mutate_route_plan_nested(
    route: dict,
    path: tuple[str, str],
    value: object,
) -> None:
    plan_path = Path(route["launchPlan"])
    plan = json.loads(plan_path.read_text())
    parent = plan[path[0]]
    parent[path[1]] = value
    plan_path.write_text(json.dumps(plan, indent=2) + "\n")


def add_campaign_summary_to_pre_eval_summary(summary: Path) -> None:
    data = json.loads(summary.read_text())
    campaign_summary = clean_campaign_summary_check(summary.parent / "combined-summary.json")
    data["options"]["campaignSummary"] = campaign_summary["summaryJson"]
    data["checks"]["campaignSummary"] = campaign_summary
    summary.write_text(json.dumps(data, indent=2) + "\n")


def clean_campaign_summary_check(path: Path) -> dict:
    manifests = []
    for campaign in ("validated-conversions", "historical-wins"):
        manifest = path.parent / f"{campaign}-manifest.json"
        manifest.write_text(json.dumps({"campaign": campaign}) + "\n")
        manifests.append(
            {
                "campaign": campaign,
                "manifest": str(manifest),
                "manifestSha256": sha256(manifest.read_bytes()).hexdigest(),
            }
        )
    path.write_text('{"validation":{"status":"ok"}}\n')
    return {
        "status": "passed",
        "summaryJson": str(path),
        "summaryJsonSha256": sha256(path.read_bytes()).hexdigest(),
        "preset": "validated-plus-historical",
        "validationStatus": "ok",
        "issues": [],
        "uniqueTasks": 18,
        "projectedPasses": 68,
        "duplicateTasks": 0,
        "duplicates": [],
        "requireNoOverlap": True,
        "expectUniqueTasks": 18,
        "expectProjectedPasses": 68,
        "expectCampaigns": ["validated-conversions", "historical-wins"],
        "expectRoutes": [
            "validated-conversions/medium-validated",
            "validated-conversions/xhigh-validated",
            "validated-conversions/xhigh-plan-first",
            "historical-wins/policy-framed",
            "historical-wins/environment-targeted",
        ],
        "expectTasks": [
            "password-recovery",
            "qemu-startup",
            "vulnerable-secret",
        ],
        "expectOwners": [
            "password-recovery=historical-wins/policy-framed",
            "qemu-startup=historical-wins/environment-targeted",
            "vulnerable-secret=historical-wins/policy-framed",
        ],
        "manifests": manifests,
    }


if __name__ == "__main__":
    unittest.main()
