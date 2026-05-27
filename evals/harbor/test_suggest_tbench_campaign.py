#!/usr/bin/env python3

from __future__ import annotations

import unittest
import sys
from pathlib import Path

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from suggest_tbench_campaign import (
    campaign_task_names,
    render_markdown,
    suggest_campaign_candidates,
)


def analysis(job_name: str, classes: dict[str, list[dict]]) -> dict:
    passes = len(classes.get("pass", []))
    scored_failures = len(classes.get("scored_fail", []))
    return {
        "job_name": job_name,
        "stats": {"passes": passes, "scored_failures": scored_failures},
        "classes": classes,
    }


class SuggestTbenchCampaignTests(unittest.TestCase):
    def test_suggests_historical_passes_not_already_in_campaign(self) -> None:
        baseline = analysis(
            "full-baseline",
            {
                "pass": [
                    {"task_name": "baseline-pass", "trial_name": "baseline-pass__1"},
                ],
                "scored_fail": [
                    {"task_name": "qemu-startup", "trial_name": "qemu-startup__1"},
                    {"task_name": "password-recovery", "trial_name": "password-recovery__1"},
                    {"task_name": "already-routed", "trial_name": "already-routed__1"},
                    {"task_name": "still-fail", "trial_name": "still-fail__1"},
                ],
                "internal_deadline_timeout": [
                    {"task_name": "qemu-startup", "trial_name": "qemu-startup__1"},
                ],
                "provider_policy_block": [
                    {"task_name": "password-recovery", "trial_name": "password-recovery__1"},
                ],
            },
        )
        evidence = [
            (
                "rerun-deadline.json",
                analysis(
                    "deadline-rerun",
                    {
                        "pass": [
                            {"task_name": "qemu-startup", "trial_name": "qemu-startup__2"},
                            {"task_name": "already-routed", "trial_name": "already-routed__2"},
                        ],
                        "scored_fail": [
                            {"task_name": "still-fail", "trial_name": "still-fail__2"},
                        ],
                    },
                ),
            ),
            (
                "plan-first.json",
                analysis(
                    "plan-first-sensitive-rerun",
                    {
                        "pass": [
                            {
                                "task_name": "password-recovery",
                                "trial_name": "password-recovery__2",
                            },
                        ],
                    },
                ),
            ),
        ]
        manifest = {
            "routes": [
                {"name": "existing", "tasks": ["already-routed"]},
            ]
        }

        report = suggest_campaign_candidates(
            baseline=baseline,
            evidence=evidence,
            existing_tasks=campaign_task_names(manifest),
        )

        self.assertEqual(
            ["password-recovery", "qemu-startup"],
            [candidate["taskName"] for candidate in report["candidates"]],
        )
        by_task = {candidate["taskName"]: candidate for candidate in report["candidates"]}
        self.assertEqual(
            "policy-framed-plan-first",
            by_task["password-recovery"]["suggestedRoute"],
        )
        self.assertEqual("environment-target", by_task["qemu-startup"]["suggestedRoute"])
        self.assertEqual(2, report["summary"]["newCandidates"])
        self.assertEqual(1, report["summary"]["excludedAlreadyRouted"])
        self.assertEqual(2, report["summary"]["evidenceReports"])

    def test_render_markdown_lists_candidate_routes(self) -> None:
        report = {
            "summary": {
                "newCandidates": 2,
                "excludedAlreadyRouted": 1,
                "baselinePasses": 50,
            },
            "candidates": [
                {
                    "taskName": "password-recovery",
                    "suggestedRoute": "policy-framed-plan-first",
                    "evidence": [{"jobName": "plan-first", "path": "plan.json"}],
                },
                {
                    "taskName": "qemu-startup",
                    "suggestedRoute": "deadline-extension",
                    "evidence": [{"jobName": "deadline", "path": "deadline.json"}],
                },
            ],
        }

        markdown = render_markdown(report)

        self.assertIn("New candidates: 2", markdown)
        self.assertIn("`password-recovery`", markdown)
        self.assertIn("`policy-framed-plan-first`", markdown)
        self.assertIn("`qemu-startup`", markdown)

    def test_routes_known_environment_and_policy_task_names(self) -> None:
        baseline = analysis(
            "full-baseline",
            {
                "scored_fail": [
                    {"task_name": "qemu-startup", "trial_name": "qemu-startup__1"},
                    {"task_name": "vulnerable-secret", "trial_name": "vulnerable-secret__1"},
                ],
            },
        )
        evidence = [
            (
                "historical.json",
                analysis(
                    "historical-full",
                    {
                        "pass": [
                            {"task_name": "qemu-startup", "trial_name": "qemu-startup__2"},
                            {
                                "task_name": "vulnerable-secret",
                                "trial_name": "vulnerable-secret__2",
                            },
                        ],
                    },
                ),
            ),
        ]

        report = suggest_campaign_candidates(
            baseline=baseline,
            evidence=evidence,
            existing_tasks=set(),
        )

        by_task = {candidate["taskName"]: candidate for candidate in report["candidates"]}
        self.assertEqual("environment-target", by_task["qemu-startup"]["suggestedRoute"])
        self.assertEqual("policy-framed", by_task["vulnerable-secret"]["suggestedRoute"])


if __name__ == "__main__":
    unittest.main()
