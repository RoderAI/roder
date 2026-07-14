#!/usr/bin/env python3
"""Build reviewable, non-overlapping route manifests for targeted Terminal-Bench reruns.

Consumes the failure records mined by ``tbench_failure_miner`` and emits, per route,
the owned tasks (no cross-route overlap), a concrete per-task reason, a conservative
expected-conversion projection, and the exact harbor run invocation for that subset.

Hard rules enforced here:

* A manifest may only contain tasks from the clean run's reward-0 set plus the
  dirty-vs-clean regressions. The reward-1 (passed) task names are read from the
  analysis JSON's ``reward_stats`` at runtime and any manifest task in that set is
  rejected (raises).
* ``capability`` tasks (no harness fix identified) never enter a runnable manifest;
  they are surfaced in the summary only.
* Manifests using a non-default timeout multiplier or a deviating auth mode are
  labelled ``local-only``; default-timeout, standard-auth manifests are
  ``leaderboard-valid-candidate``.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_failure_miner import (  # noqa: E402
    FailureRecord,
    build_failure_records,
    clean_run_fail_tasks,
    clean_run_pass_tasks,
    load_json,
    miner_evidence_by_task,
    redact,
    regressed_task_names,
)
from tbench_score_routes_render import (  # noqa: E402
    render_route_markdown,
    render_summary_markdown,
)
from tbench_route_constants import (  # noqa: E402
    BLOCKED_HOST_NOTE,
    BLOCKED_ROUTES,
    CLEAN_RUN_SOFT_WINDOW_SEC,
    HARBOR_AGENT,
    HARBOR_DATASET,
    HARBOR_JOBS_DIR,
    HARBOR_MODEL,
    ROUTE_AK,
    ROUTE_CLASSES,
    ROUTE_DESCRIPTIONS,
    SUMMARY_ONLY_ROUTES,
    TRACK_LEADERBOARD,
    TRACK_LOCAL_ONLY,
    normalize_task_name,
)

DEFAULT_AGENT_TIMEOUT_MULTIPLIER = 1.0
DEFAULT_AUTH_MODE = "standard"
STANDARD_AUTH_MODES = frozenset({"standard", "oauth", "default"})
MAX_REASON_CHARS = 360


def projected_conversion(record: FailureRecord) -> bool:
    """Conservative expected-conversion test.

    A task only projects as a conversion when it was a near miss, and for the
    deadline-extension route additionally requires the task's Terminal-Bench
    default window to exceed the clean run's shortened window (else restoring the
    window buys nothing).
    """
    if not record.near_miss:
        return False
    if record.route == "deadline-extension":
        return record.task_timeout_sec is not None and record.task_timeout_sec > CLEAN_RUN_SOFT_WINDOW_SEC
    return True


def resolve_track(*, agent_timeout_multiplier: float, auth_mode: str) -> str:
    """local-only for any timeout/auth deviation; leaderboard candidate otherwise."""
    if agent_timeout_multiplier not in (None, 1.0):
        return TRACK_LOCAL_ONLY
    if auth_mode not in STANDARD_AUTH_MODES:
        return TRACK_LOCAL_ONLY
    return TRACK_LEADERBOARD


def guard_manifest_tasks(
    *,
    tasks: list[str],
    pass_tasks: set[str],
    allowed_tasks: set[str],
    route: str,
) -> None:
    """Reject any manifest task that passed the clean run or is outside scope."""
    illegal_pass = sorted(t for t in tasks if t in pass_tasks)
    if illegal_pass:
        raise ValueError(
            f"route {route!r} would include clean-run pass task(s): {', '.join(illegal_pass)}"
        )
    outside = sorted(t for t in tasks if t not in allowed_tasks)
    if outside:
        raise ValueError(
            f"route {route!r} includes task(s) outside the reward-0 + regression scope: "
            f"{', '.join(outside)}"
        )


def _clip_reason(text: str) -> str:
    clipped = redact(str(text)).strip()
    if len(clipped) <= MAX_REASON_CHARS:
        return clipped
    return clipped[: MAX_REASON_CHARS - 1].rstrip() + "…"


def task_reasons(miner_evidence: Any) -> dict[str, str]:
    """task -> concrete harness-fix reason, drawn from miner evidence."""
    reasons: dict[str, str] = {}
    for task, record in miner_evidence_by_task(miner_evidence).items():
        fix = record.get("harness_fix")
        if isinstance(fix, str) and fix.strip():
            reasons[task] = _clip_reason(fix)
    return reasons


def reason_for(record: FailureRecord, reasons: dict[str, str]) -> str:
    if record.task in reasons:
        return reasons[record.task]
    if record.verifier_failure:
        return record.verifier_failure
    if record.evidence:
        return record.evidence[0]
    return ROUTE_DESCRIPTIONS.get(record.route, record.route)


def harbor_command(
    *,
    include_task_names: list[str],
    ak: dict[str, str],
    agent_timeout_multiplier: float,
    job_name: str,
    n_concurrent: int,
) -> str:
    lines = [
        'PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}" harbor run \\',
        f"  -d {HARBOR_DATASET} \\",
        f"  -a {HARBOR_AGENT} \\",
        f"  -m {HARBOR_MODEL} \\",
    ]
    for task in include_task_names:
        lines.append(f"  --include-task-name {task} \\")
    for key, value in ak.items():
        lines.append(f"  --ak {key}={value} \\")
    lines.append(f"  --agent-timeout-multiplier {agent_timeout_multiplier} \\")
    lines.append(f"  --job-name {job_name} \\")
    lines.append(f"  --jobs-dir {HARBOR_JOBS_DIR} \\")
    lines.append("  --n-attempts 1 \\")
    lines.append(f"  --n-concurrent {n_concurrent} \\")
    lines.append("  --yes")
    return "\n".join(lines)


def build_route_manifest(
    *,
    route: str,
    records: list[FailureRecord],
    reasons: dict[str, str],
    track: str,
    agent_timeout_multiplier: float,
    auth_mode: str,
) -> dict[str, Any]:
    blocked = route in BLOCKED_ROUTES
    runnable = not blocked
    tasks = [r.task for r in records]
    n_concurrent = max(1, min(4, len(tasks)))
    job_name = f"roder-tbench-21-score-{route}-v1"
    ak = dict(ROUTE_AK.get(route, {"reasoning": "xhigh"}))

    task_entries: list[dict[str, Any]] = []
    for record in records:
        converts = projected_conversion(record)
        task_entries.append(
            {
                "task": record.task,
                "reason": reason_for(record, reasons),
                "dominantCause": record.dominant_cause,
                "nearMiss": record.near_miss,
                "regression": record.regression,
                "longerWindowWouldHelp": record.longer_window_would_help,
                "taskTimeoutSec": record.task_timeout_sec,
                "projectedConversion": converts,
            }
        )

    projected = sum(1 for e in task_entries if e["projectedConversion"])
    manifest: dict[str, Any] = {
        "route": route,
        "description": ROUTE_DESCRIPTIONS.get(route, ""),
        "runnable": runnable,
        "blocked": blocked,
        "track": track,
        "projection": {
            "tasks": len(task_entries),
            "nearMisses": sum(1 for e in task_entries if e["nearMiss"]),
            "regressions": sum(1 for e in task_entries if e["regression"]),
            "projectedConversions": projected,
        },
        "tasks": task_entries,
        "harborInvocation": {
            "dataset": HARBOR_DATASET,
            "agent": HARBOR_AGENT,
            "model": HARBOR_MODEL,
            "includeTaskNames": tasks,
            "ak": ak,
            "agentTimeoutMultiplier": agent_timeout_multiplier,
            "authMode": auth_mode,
            "nAttempts": 1,
            "nConcurrent": n_concurrent,
            "jobName": job_name,
            "jobsDir": HARBOR_JOBS_DIR,
            "command": harbor_command(
                include_task_names=tasks,
                ak=ak,
                agent_timeout_multiplier=agent_timeout_multiplier,
                job_name=job_name,
                n_concurrent=n_concurrent,
            ),
        },
    }
    if blocked:
        manifest["blockedReason"] = BLOCKED_HOST_NOTE
    return manifest


def build_manifests(
    *,
    analysis: dict[str, Any],
    comparison: dict[str, Any] | None,
    miner_evidence: Any,
    agent_timeout_multiplier: float = DEFAULT_AGENT_TIMEOUT_MULTIPLIER,
    auth_mode: str = DEFAULT_AUTH_MODE,
) -> dict[str, Any]:
    records = build_failure_records(
        analysis=analysis, comparison=comparison, miner_evidence=miner_evidence
    )
    reasons = task_reasons(miner_evidence)
    pass_tasks = clean_run_pass_tasks(analysis)
    fail_tasks = clean_run_fail_tasks(analysis)
    allowed_tasks = fail_tasks | regressed_task_names(comparison)
    track = resolve_track(agent_timeout_multiplier=agent_timeout_multiplier, auth_mode=auth_mode)

    by_route: dict[str, list[FailureRecord]] = {route: [] for route in ROUTE_CLASSES}
    for record in records:
        by_route[record.route].append(record)

    # Ownership is one route per task by construction; assert no task leaks across routes.
    seen: dict[str, str] = {}
    for route, route_records in by_route.items():
        for record in route_records:
            if record.task in seen:
                raise ValueError(
                    f"task {record.task!r} owned by both {seen[record.task]!r} and {route!r}"
                )
            seen[record.task] = route

    manifests: dict[str, dict[str, Any]] = {}
    capability_tasks: list[dict[str, Any]] = []
    for route in ROUTE_CLASSES:
        route_records = by_route.get(route, [])
        if not route_records:
            continue
        if route in SUMMARY_ONLY_ROUTES:
            for record in route_records:
                capability_tasks.append(
                    {
                        "task": record.task,
                        "reason": reason_for(record, reasons),
                        "dominantCause": record.dominant_cause,
                        "nearMiss": record.near_miss,
                        "regression": record.regression,
                    }
                )
            continue
        guard_manifest_tasks(
            tasks=[r.task for r in route_records],
            pass_tasks=pass_tasks,
            allowed_tasks=allowed_tasks,
            route=route,
        )
        manifests[route] = build_route_manifest(
            route=route,
            records=route_records,
            reasons=reasons,
            track=track,
            agent_timeout_multiplier=agent_timeout_multiplier,
            auth_mode=auth_mode,
        )

    runnable_conversions = sum(
        m["projection"]["projectedConversions"] for m in manifests.values() if m["runnable"]
    )
    blocked_conversions = sum(
        m["projection"]["projectedConversions"] for m in manifests.values() if not m["runnable"]
    )
    manifested_tasks = sum(m["projection"]["tasks"] for m in manifests.values())

    summary = {
        "job_name": analysis.get("job_name"),
        "cleanRunSoftWindowSec": CLEAN_RUN_SOFT_WINDOW_SEC,
        "track": track,
        "agentTimeoutMultiplier": agent_timeout_multiplier,
        "authMode": auth_mode,
        "counts": {
            "rewardZeroTasks": len(fail_tasks),
            "regressions": len(regressed_task_names(comparison)),
            "manifestedTasks": manifested_tasks,
            "capabilityTasks": len(capability_tasks),
            "runnableProjectedConversions": runnable_conversions,
            "blockedProjectedConversions": blocked_conversions,
        },
        "routes": {
            route: {
                "tasks": manifest["projection"]["tasks"],
                "runnable": manifest["runnable"],
                "blocked": manifest["blocked"],
                "projectedConversions": manifest["projection"]["projectedConversions"],
                "track": manifest["track"],
            }
            for route, manifest in manifests.items()
        },
    }
    return {"summary": summary, "manifests": manifests, "capabilityTasks": capability_tasks}


def write_outputs(result: dict[str, Any], output_dir: Path) -> list[Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for route, manifest in result["manifests"].items():
        manifest_path = output_dir / f"{route}-manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
        route_md = output_dir / f"{route}.md"
        route_md.write_text(render_route_markdown(manifest))
        written.extend([manifest_path, route_md])
    summary_json = output_dir / "score-routes.json"
    summary_json.write_text(json.dumps(result["summary"], indent=2) + "\n")
    summary_md = output_dir / "summary.md"
    summary_md.write_text(render_summary_markdown(result))
    written.extend([summary_json, summary_md])
    return written


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analysis", type=Path, required=True)
    parser.add_argument("--comparison", type=Path)
    parser.add_argument("--miner-evidence", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--agent-timeout-multiplier",
        type=float,
        default=DEFAULT_AGENT_TIMEOUT_MULTIPLIER,
        help="Non-default values mark every manifest local-only.",
    )
    parser.add_argument(
        "--auth-mode",
        default=DEFAULT_AUTH_MODE,
        help="Deviating auth (e.g. access-token-only) marks every manifest local-only.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        analysis = load_json(args.analysis)
        comparison = load_json(args.comparison) if args.comparison else None
        miner_evidence = load_json(args.miner_evidence) if args.miner_evidence else None
        result = build_manifests(
            analysis=analysis,
            comparison=comparison,
            miner_evidence=miner_evidence,
            agent_timeout_multiplier=args.agent_timeout_multiplier,
            auth_mode=args.auth_mode,
        )
        written = write_outputs(result, args.output_dir)
    except Exception as exc:  # noqa: BLE001 - CLI boundary
        print(f"tbench_score_routes: {exc}", file=sys.stderr)
        return 2

    counts = result["summary"]["counts"]
    print(
        "Wrote {n} files to {dir}: {routes} routes, {conv} runnable projected conversions".format(
            n=len(written),
            dir=args.output_dir,
            routes=len(result["manifests"]),
            conv=counts["runnableProjectedConversions"],
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
