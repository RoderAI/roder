#!/usr/bin/env python3
"""Suggest historical Terminal-Bench wins missing from a generated campaign."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from compare_tbench_runs import load_analysis, task_states  # noqa: E402


Evidence = tuple[str, dict[str, Any]]


def campaign_task_names(manifest: dict[str, Any]) -> set[str]:
    tasks: set[str] = set()
    routes = manifest.get("routes")
    if not isinstance(routes, list):
        return tasks
    for route in routes:
        if not isinstance(route, dict):
            continue
        route_tasks = route.get("tasks")
        if not isinstance(route_tasks, list):
            continue
        tasks.update(str(task) for task in route_tasks if task)
    return tasks


def suggest_campaign_candidates(
    *,
    baseline: dict[str, Any],
    evidence: list[Evidence] | tuple[Evidence, ...],
    existing_tasks: set[str] | frozenset[str],
) -> dict[str, Any]:
    baseline_states = task_states(baseline)
    by_task: dict[str, dict[str, Any]] = {}
    excluded_already_routed: set[str] = set()
    evidence_reports = 0

    for path, evidence_analysis in evidence:
        states = task_states(evidence_analysis)
        report_added_evidence = False
        for task_name, state in sorted(states.items()):
            baseline_state = baseline_states.get(task_name)
            if baseline_state is None or baseline_state.reward != 0.0:
                continue
            if state.reward != 1.0:
                continue
            if task_name in existing_tasks:
                excluded_already_routed.add(task_name)
                continue
            report_added_evidence = True
            candidate = by_task.setdefault(
                task_name,
                {
                    "taskName": task_name,
                    "baselineClasses": sorted(baseline_state.classes),
                    "suggestedRoute": suggested_route(
                        task_name=task_name,
                        baseline_classes=baseline_state.classes,
                        evidence_job_names=[],
                    ),
                    "evidence": [],
                },
            )
            candidate["evidence"].append(
                {
                    "path": str(path),
                    "jobName": str(evidence_analysis.get("job_name") or ""),
                    "trialName": state.trial_name,
                    "classes": sorted(state.classes),
                    "reward": state.reward,
                }
            )
            candidate["suggestedRoute"] = suggested_route(
                task_name=task_name,
                baseline_classes=baseline_state.classes,
                evidence_job_names=[
                    item["jobName"] for item in candidate["evidence"]
                ],
            )
        if report_added_evidence:
            evidence_reports += 1

    candidates = [by_task[task] for task in sorted(by_task)]
    return {
        "summary": {
            "baselineJob": baseline.get("job_name"),
            "baselinePasses": stat_int(baseline, "passes"),
            "baselineScoredFailures": stat_int(baseline, "scored_failures"),
            "existingCampaignTasks": len(existing_tasks),
            "newCandidates": len(candidates),
            "excludedAlreadyRouted": len(excluded_already_routed),
            "evidenceReports": evidence_reports,
        },
        "candidates": candidates,
        "excludedAlreadyRouted": sorted(excluded_already_routed),
    }


def suggested_route(
    *,
    task_name: str,
    baseline_classes: set[str],
    evidence_job_names: list[str],
) -> str:
    saw_plan_first = any("plan-first" in name for name in evidence_job_names)
    if task_name in {
        "install-windows-3.11",
        "qemu-alpine-ssh",
        "qemu-startup",
        "train-fasttext",
    }:
        return "environment-target"
    if task_name in {
        "crack-7z-hash",
        "git-leak-recovery",
        "model-extraction-relu-logits",
        "password-recovery",
        "vulnerable-secret",
    } and saw_plan_first:
        return "policy-framed-plan-first"
    if task_name in {
        "crack-7z-hash",
        "git-leak-recovery",
        "model-extraction-relu-logits",
        "password-recovery",
        "vulnerable-secret",
    }:
        return "policy-framed"
    if "provider_policy_block" in baseline_classes and saw_plan_first:
        return "policy-framed-plan-first"
    if saw_plan_first:
        return "plan-first"
    if baseline_classes.intersection(
        {"internal_deadline_timeout", "soft_timeout", "soft_timeout_fail"}
    ):
        return "deadline-extension"
    if "provider_policy_block" in baseline_classes:
        return "policy-framed"
    return "historical-win"


def stat_int(analysis: dict[str, Any], name: str) -> int | None:
    stats = analysis.get("stats")
    if not isinstance(stats, dict):
        return None
    value = stats.get(name)
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def render_markdown(report: dict[str, Any]) -> str:
    summary = report.get("summary") if isinstance(report.get("summary"), dict) else {}
    lines = [
        "# TBench Campaign Suggestions",
        "",
        f"- Baseline job: `{summary.get('baselineJob') or '<unknown>'}`",
        f"- Baseline passes: {summary.get('baselinePasses')}",
        f"- Existing campaign tasks: {summary.get('existingCampaignTasks')}",
        f"- New candidates: {summary.get('newCandidates')}",
        f"- Excluded already-routed wins: {summary.get('excludedAlreadyRouted')}",
        "",
        "| Task | Suggested route | Evidence |",
        "| --- | --- | --- |",
    ]
    candidates = report.get("candidates")
    if not isinstance(candidates, list) or not candidates:
        lines.append("| None | - | - |")
        return "\n".join(lines).rstrip() + "\n"
    for candidate in candidates:
        if not isinstance(candidate, dict):
            continue
        evidence = candidate.get("evidence")
        evidence_bits = []
        if isinstance(evidence, list):
            for item in evidence:
                if isinstance(item, dict):
                    job = item.get("jobName") or Path(str(item.get("path") or "")).stem
                    evidence_bits.append(f"`{job}`")
        lines.append(
            "| `{}` | `{}` | {} |".format(
                candidate.get("taskName"),
                candidate.get("suggestedRoute"),
                ", ".join(evidence_bits) if evidence_bits else "-",
            )
        )
    return "\n".join(lines).rstrip() + "\n"


def load_campaign_manifest(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, action="append", required=True)
    parser.add_argument("--campaign-manifest", type=Path)
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        baseline = load_analysis(args.baseline)
        evidence = [(str(path), load_analysis(path)) for path in args.evidence]
        manifest = load_campaign_manifest(args.campaign_manifest)
        report = suggest_campaign_candidates(
            baseline=baseline,
            evidence=evidence,
            existing_tasks=campaign_task_names(manifest),
        )
    except Exception as exc:
        print(f"suggest_tbench_campaign: {exc}", file=sys.stderr)
        return 2

    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(report, indent=2) + "\n")

    markdown = render_markdown(report)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown)

    if not args.json_path and not args.markdown:
        print(markdown, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
