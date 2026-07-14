#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

import tbench_route_test_helpers as fixtures  # noqa: E402
from tbench_failure_miner import FailureRecord  # noqa: E402
from tbench_score_routes import (  # noqa: E402
    build_manifests,
    guard_manifest_tasks,
    projected_conversion,
    resolve_track,
    write_outputs,
)


def _record(route: str, *, near_miss: bool, timeout: int | None) -> FailureRecord:
    return FailureRecord(
        task="synthetic",
        analyzer_classes=[],
        dominant_cause="test",
        route=route,
        regression=False,
        near_miss=near_miss,
        longer_window_would_help=True,
        task_timeout_sec=timeout,
        verifier_failure=None,
    )


class ProjectionTest(unittest.TestCase):
    def test_deadline_extension_requires_near_miss_and_window(self) -> None:
        self.assertTrue(projected_conversion(_record("deadline-extension", near_miss=True, timeout=900)))
        self.assertFalse(projected_conversion(_record("deadline-extension", near_miss=True, timeout=700)))
        self.assertFalse(projected_conversion(_record("deadline-extension", near_miss=False, timeout=3600)))

    def test_non_deadline_route_projects_on_near_miss_alone(self) -> None:
        self.assertTrue(projected_conversion(_record("task-contract", near_miss=True, timeout=None)))
        self.assertFalse(projected_conversion(_record("policy-framed", near_miss=False, timeout=900)))


class TrackTest(unittest.TestCase):
    def test_default_is_leaderboard_candidate(self) -> None:
        self.assertEqual(
            resolve_track(agent_timeout_multiplier=1.0, auth_mode="standard"),
            "leaderboard-valid-candidate",
        )

    def test_timeout_multiplier_deviation_is_local_only(self) -> None:
        self.assertEqual(
            resolve_track(agent_timeout_multiplier=2.0, auth_mode="standard"), "local-only"
        )

    def test_auth_deviation_is_local_only(self) -> None:
        self.assertEqual(
            resolve_track(agent_timeout_multiplier=1.0, auth_mode="access-token"), "local-only"
        )


class GuardTest(unittest.TestCase):
    def test_rejects_clean_run_pass_task(self) -> None:
        with self.assertRaises(ValueError):
            guard_manifest_tasks(
                tasks=["crack-7z-hash", "extract-elf"],
                pass_tasks={"extract-elf"},
                allowed_tasks={"crack-7z-hash", "extract-elf"},
                route="deadline-extension",
            )

    def test_rejects_task_outside_scope(self) -> None:
        with self.assertRaises(ValueError):
            guard_manifest_tasks(
                tasks=["some-random-task"],
                pass_tasks=set(),
                allowed_tasks={"crack-7z-hash"},
                route="deadline-extension",
            )

    def test_accepts_in_scope_tasks(self) -> None:
        guard_manifest_tasks(
            tasks=["crack-7z-hash"],
            pass_tasks={"extract-elf"},
            allowed_tasks={"crack-7z-hash", "regex-chess"},
            route="deadline-extension",
        )


class BuildManifestsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.result = build_manifests(
            analysis=fixtures.analysis(),
            comparison=fixtures.comparison(),
            miner_evidence=fixtures.miner_evidence(),
        )
        self.manifests = self.result["manifests"]
        self.summary = self.result["summary"]

    def test_capability_is_summary_only(self) -> None:
        self.assertNotIn("capability", self.manifests)
        capability_tasks = {t["task"] for t in self.result["capabilityTasks"]}
        self.assertEqual(capability_tasks, {"make-doom-for-mips"})

    def test_runnable_routes_present(self) -> None:
        self.assertEqual(
            set(self.manifests),
            {
                "deadline-extension",
                "task-contract",
                "policy-framed",
                "historical-regression",
                "environment-service",
            },
        )

    def test_no_task_overlap_across_routes(self) -> None:
        seen: set[str] = set()
        for manifest in self.manifests.values():
            tasks = {t["task"] for t in manifest["tasks"]}
            self.assertTrue(seen.isdisjoint(tasks), f"overlap: {seen & tasks}")
            seen |= tasks

    def test_environment_service_is_blocked_not_runnable(self) -> None:
        env = self.manifests["environment-service"]
        self.assertFalse(env["runnable"])
        self.assertTrue(env["blocked"])
        self.assertIn("blockedReason", env)

    def test_track_labeling_default(self) -> None:
        self.assertEqual(self.summary["track"], "leaderboard-valid-candidate")
        for manifest in self.manifests.values():
            self.assertEqual(manifest["track"], "leaderboard-valid-candidate")

    def test_local_only_track_on_multiplier(self) -> None:
        result = build_manifests(
            analysis=fixtures.analysis(),
            comparison=fixtures.comparison(),
            miner_evidence=fixtures.miner_evidence(),
            agent_timeout_multiplier=2.0,
        )
        self.assertEqual(result["summary"]["track"], "local-only")

    def test_projection_counts(self) -> None:
        # Runnable near-miss conversions: crack-7z-hash, pytorch-model-recovery, headless-terminal.
        self.assertEqual(self.summary["counts"]["runnableProjectedConversions"], 3)
        # Blocked env-service near miss: torch-pipeline-parallelism.
        self.assertEqual(self.summary["counts"]["blockedProjectedConversions"], 1)

    def test_no_manifest_task_passed_clean_run(self) -> None:
        pass_tasks = set(fixtures.analysis()["stats"]["harbor"]["evals"][fixtures.EVAL_KEY]["reward_stats"]["reward"]["1.0"])
        pass_names = {t.split("__", 1)[0] for t in pass_tasks}
        for manifest in self.manifests.values():
            for entry in manifest["tasks"]:
                self.assertNotIn(entry["task"], pass_names)

    def test_harbor_invocation_targets_only_route_tasks(self) -> None:
        dl = self.manifests["deadline-extension"]
        include = dl["harborInvocation"]["includeTaskNames"]
        self.assertEqual(set(include), {"crack-7z-hash", "regex-chess"})
        self.assertIn("--include-task-name crack-7z-hash", dl["harborInvocation"]["command"])
        self.assertIn("reasoning=xhigh", dl["harborInvocation"]["command"])

    def test_write_outputs_emits_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            write_outputs(self.result, out)
            self.assertTrue((out / "summary.md").exists())
            self.assertTrue((out / "score-routes.json").exists())
            self.assertTrue((out / "deadline-extension-manifest.json").exists())
            self.assertTrue((out / "deadline-extension.md").exists())
            data = json.loads((out / "deadline-extension-manifest.json").read_text())
            self.assertEqual(data["route"], "deadline-extension")


if __name__ == "__main__":
    unittest.main()
