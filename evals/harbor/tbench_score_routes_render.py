#!/usr/bin/env python3
"""Markdown renderers for ``tbench_score_routes`` route manifests and the combined summary.

Pure formatting: kept separate from ``tbench_score_routes`` so the manifest-building
logic stays under the 500-line split threshold and can be reasoned about on its own.
"""

from __future__ import annotations

from typing import Any

from tbench_route_constants import CAPABILITY_NOTE


def render_route_markdown(manifest: dict[str, Any]) -> str:
    proj = manifest["projection"]
    inv = manifest["harborInvocation"]
    lines = [
        f"# Route: {manifest['route']}",
        "",
        manifest["description"],
        "",
        f"- Track: `{manifest['track']}`",
        f"- Runnable: {'yes' if manifest['runnable'] else 'no'}",
    ]
    if manifest.get("blocked"):
        lines.append(f"- Blocked: {manifest.get('blockedReason')}")
    lines += [
        f"- Tasks: {proj['tasks']} ({proj['nearMisses']} near-miss, {proj['regressions']} regressions)",
        f"- Projected conversions (conservative): {proj['projectedConversions']}",
        "",
        "## Tasks",
        "",
        "| Task | Timeout (s) | Near-miss | Regression | Converts | Reason |",
        "| --- | ---: | :---: | :---: | :---: | --- |",
    ]
    for entry in manifest["tasks"]:
        lines.append(
            "| `{task}` | {to} | {nm} | {reg} | {cv} | {reason} |".format(
                task=entry["task"],
                to=entry["taskTimeoutSec"] if entry["taskTimeoutSec"] is not None else "?",
                nm="yes" if entry["nearMiss"] else "-",
                reg="yes" if entry["regression"] else "-",
                cv="yes" if entry["projectedConversion"] else "-",
                reason=entry["reason"].replace("|", "\\|"),
            )
        )
    lines += [
        "",
        "## Harbor invocation",
        "",
        f"Job: `{inv['jobName']}` | agent-timeout-multiplier: {inv['agentTimeoutMultiplier']} | auth: `{inv['authMode']}`",
        "",
        "```sh",
        inv["command"],
        "```",
        "",
    ]
    return "\n".join(lines).rstrip() + "\n"


def render_summary_markdown(result: dict[str, Any]) -> str:
    summary = result["summary"]
    counts = summary["counts"]
    lines = [
        "# TBench Score Routes",
        "",
        f"- Job: `{summary.get('job_name') or '<unknown>'}`",
        f"- Track: `{summary['track']}` (agent-timeout-multiplier {summary['agentTimeoutMultiplier']}, auth `{summary['authMode']}`)",
        f"- Reward-0 tasks: {counts['rewardZeroTasks']} | regressions: {counts['regressions']}",
        f"- Tasks in runnable/blocked manifests: {counts['manifestedTasks']} | capability (summary-only): {counts['capabilityTasks']}",
        f"- Runnable projected conversions (conservative): {counts['runnableProjectedConversions']}",
        f"- Blocked-on-host projected conversions: {counts['blockedProjectedConversions']}",
        "",
        "## Routes",
        "",
        "| Route | Tasks | Runnable | Projected conversions | Track |",
        "| --- | ---: | :---: | ---: | --- |",
    ]
    for route, info in sorted(summary["routes"].items()):
        runnable = "yes"
        if info["blocked"]:
            runnable = "blocked-on-host"
        elif not info["runnable"]:
            runnable = "no"
        lines.append(
            "| `{route}` | {tasks} | {runnable} | {conv} | `{track}` |".format(
                route=route,
                tasks=info["tasks"],
                runnable=runnable,
                conv=info["projectedConversions"],
                track=info["track"],
            )
        )
    capability = result.get("capabilityTasks", [])
    lines += [
        "",
        "## Capability (no harness fix; guidance-level only)",
        "",
        CAPABILITY_NOTE,
        "",
        "| Task | Cause | Near-miss | Reason |",
        "| --- | --- | :---: | --- |",
    ]
    if not capability:
        lines.append("| None | - | - | - |")
    for entry in capability:
        lines.append(
            "| `{task}` | {cause} | {nm} | {reason} |".format(
                task=entry["task"],
                cause=entry["dominantCause"],
                nm="yes" if entry["nearMiss"] else "-",
                reason=entry["reason"].replace("|", "\\|"),
            )
        )
    return "\n".join(lines).rstrip() + "\n"
