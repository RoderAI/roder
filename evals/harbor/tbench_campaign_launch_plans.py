"""Launch-plan validation for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Protocol

from validate_tbench_launch_plan import validate_plan


CONSISTENT_ROUTE_PLAN_FIELDS = (
    "launchStatus",
    "dryRun",
    "wouldRunHarbor",
    "preEvalSummary",
    "preEvalSummarySha256",
    "preEvalOutputDir",
    "preEvalRanHere",
    "requireCampaignSummary",
    "requireAnalysis",
    "pullPreflight",
    "offlinePreflight",
    "maxPreEvalAgeSeconds",
)
CONSISTENT_CAMPAIGN_SUMMARY_FIELDS = (
    "summaryJson",
    "summaryJsonSha256",
    "preset",
    "validationStatus",
    "uniqueTasks",
    "projectedPasses",
    "duplicateTasks",
)


class IssueSink(Protocol):
    def add(self, issue: str) -> None: ...


class LaunchPlanSet:
    def __init__(self) -> None:
        self.expected: dict[str, Any] = {}

    def validate_consistency(
        self,
        result: IssueSink,
        name: str,
        *,
        plan: dict[str, Any],
    ) -> None:
        for field in CONSISTENT_ROUTE_PLAN_FIELDS:
            self._validate_value(result, name, field, plan.get(field))
        campaign_summary = plan.get("campaignSummary")
        if isinstance(campaign_summary, dict):
            for field in CONSISTENT_CAMPAIGN_SUMMARY_FIELDS:
                self._validate_value(
                    result,
                    name,
                    f"campaignSummary.{field}",
                    campaign_summary.get(field),
                )

    def _validate_value(
        self,
        result: IssueSink,
        name: str,
        key: str,
        value: Any,
    ) -> None:
        current = self.expected.get(key)
        if key not in self.expected:
            self.expected[key] = value
        elif value != current:
            result.add(f"route {name} launchPlan {key} mismatch")


def validate_route_launch_plan(
    result: IssueSink,
    name: str,
    *,
    route: dict[str, Any],
    allow_dry_run: bool,
    plan_set: LaunchPlanSet | None = None,
) -> None:
    plan_path = route.get("launchPlan")
    if not isinstance(plan_path, str) or not plan_path:
        result.add(f"route {name} launchPlan is missing")
        return
    try:
        plan = load_json(Path(plan_path))
    except Exception as exc:
        result.add(f"route {name} launchPlan cannot be read: {exc}")
        return

    validate_route_fields(result, name, route=route, plan=plan)
    if plan_set is not None:
        plan_set.validate_consistency(result, name, plan=plan)
    validation = validate_plan(
        plan,
        require_ready=not allow_dry_run,
        allow_dry_run=allow_dry_run,
        require_image_preflight=True,
        verify_harbor_config=True,
        verify_image_manifest=True,
    )
    for issue in validation.issues:
        result.add(f"route {name} launchPlan validation: {issue}")


def validate_route_fields(
    result: IssueSink,
    name: str,
    *,
    route: dict[str, Any],
    plan: dict[str, Any],
) -> None:
    expected_fields = (
        ("harborConfig", "config"),
        ("jobDir", "jobDir"),
        ("analysisJson", "analysisJson"),
        ("analysisMarkdown", "analysisMarkdown"),
    )
    for plan_field, route_field in expected_fields:
        if plan.get(plan_field) != route.get(route_field):
            result.add(f"route {name} launchPlan {plan_field} mismatch")

    image_preflight = plan.get("imagePreflight")
    expected_manifest = route.get("imageManifest")
    if plan.get("imagePreflightSource") != "route_manifest":
        result.add(f"route {name} launchPlan imagePreflightSource mismatch")
    if not isinstance(image_preflight, dict):
        result.add(f"route {name} launchPlan imagePreflight is missing")
    elif image_preflight.get("manifest") != expected_manifest:
        result.add(f"route {name} launchPlan imagePreflight manifest mismatch")


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data
