#!/usr/bin/env python3
"""Mine reward-0 Terminal-Bench trials into actionable, route-labelled failure records.

Combines three inputs into one per-task record:

* an analyzer JSON (``analyze_tbench_run`` output) for reward and analyzer classes;
* optionally a comparison JSON (``compare_tbench_runs`` output) for the regression flag;
* optionally a miner-evidence JSON (the hand-verified ``miners.json`` format) for the
  dominant actionable cause, route, near-miss flag, and trajectory evidence strings.

The record's ``route`` comes from miner evidence when present (it is the ground truth
for route assignment) and otherwise falls back to an analyzer-class heuristic.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_route_constants import (  # noqa: E402
    CLEAN_RUN_SOFT_WINDOW_SEC,
    ROUTE_CLASSES,
    normalize_task_name,
    route_from_classes,
    task_timeout,
)

# Longest evidence string kept per record, so route manifests stay reviewable.
MAX_EVIDENCE_ITEMS = 6
MAX_EVIDENCE_CHARS = 320

# Best-effort redaction of secret-bearing values that could ride along in evidence.
_REDACTIONS: tuple[tuple[re.Pattern[str], str], ...] = (
    (re.compile(r"(?i)\b(bearer|token|api[_-]?key|secret|password|refresh_token)\b\s*[:=]\s*\S+"), r"\1=<redacted>"),
    (re.compile(r"\b(sk|pk|ghp|gho|xox[baprs])-[A-Za-z0-9_\-]{8,}"), "<redacted-token>"),
    (re.compile(r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{6,}"), "<redacted-jwt>"),
)


def redact(text: str) -> str:
    out = str(text)
    for pattern, replacement in _REDACTIONS:
        out = pattern.sub(replacement, out)
    return out


def _clip(text: str) -> str:
    redacted = redact(text)
    if len(redacted) <= MAX_EVIDENCE_CHARS:
        return redacted
    return redacted[: MAX_EVIDENCE_CHARS - 1].rstrip() + "…"


@dataclass
class FailureRecord:
    task: str
    analyzer_classes: list[str]
    dominant_cause: str
    route: str
    regression: bool
    near_miss: bool
    longer_window_would_help: bool
    task_timeout_sec: int | None
    verifier_failure: str | None
    evidence: list[str] = field(default_factory=list)
    has_miner_evidence: bool = False

    def to_json(self) -> dict[str, Any]:
        return {
            "task": self.task,
            "analyzerClasses": self.analyzer_classes,
            "dominantCause": self.dominant_cause,
            "route": self.route,
            "regression": self.regression,
            "nearMiss": self.near_miss,
            "longerWindowWouldHelp": self.longer_window_would_help,
            "taskTimeoutSec": self.task_timeout_sec,
            "verifierFailure": self.verifier_failure,
            "evidence": self.evidence,
            "hasMinerEvidence": self.has_miner_evidence,
        }


def load_json(path: Path) -> Any:
    return json.loads(Path(path).read_text())


def _first_eval_stats(analysis: dict[str, Any]) -> dict[str, Any]:
    stats = analysis.get("stats")
    harbor = stats.get("harbor") if isinstance(stats, dict) else None
    evals = harbor.get("evals") if isinstance(harbor, dict) else None
    if isinstance(evals, dict) and evals:
        first = next(iter(evals.values()))
        if isinstance(first, dict):
            return first
    return {}


def _reward_task_names(analysis: dict[str, Any], reward_key: str) -> set[str]:
    """Task names at a given reward from ``reward_stats``; falls back to ``classes``."""
    reward_stats = _first_eval_stats(analysis).get("reward_stats")
    if isinstance(reward_stats, dict):
        reward = reward_stats.get("reward")
        if isinstance(reward, dict) and reward_key in reward:
            trials = reward.get(reward_key) or []
            return {normalize_task_name(t) for t in trials}
    # Fallback: derive from the analyzer classes map.
    classes = analysis.get("classes")
    class_name = "pass" if reward_key == "1.0" else "scored_fail"
    names: set[str] = set()
    if isinstance(classes, dict):
        for entry in classes.get(class_name, []) or []:
            if isinstance(entry, dict) and entry.get("task_name"):
                names.add(normalize_task_name(entry["task_name"]))
    return names


def clean_run_pass_tasks(analysis: dict[str, Any]) -> set[str]:
    """The reward-1 (passed) task names — the guard set no manifest may include."""
    return _reward_task_names(analysis, "1.0")


def clean_run_fail_tasks(analysis: dict[str, Any]) -> set[str]:
    """The reward-0 task names — the eligible improvement scope."""
    return _reward_task_names(analysis, "0.0")


def task_classes_map(analysis: dict[str, Any]) -> dict[str, set[str]]:
    """task name -> union of analyzer classes across every class bucket it appears in."""
    result: dict[str, set[str]] = {}
    classes = analysis.get("classes")
    if not isinstance(classes, dict):
        return result
    for class_name, entries in classes.items():
        if not isinstance(entries, list):
            continue
        for entry in entries:
            if isinstance(entry, dict) and entry.get("task_name"):
                task = normalize_task_name(entry["task_name"])
                result.setdefault(task, set()).add(class_name)
    return result


def regressed_task_names(comparison: dict[str, Any] | None) -> set[str]:
    if not isinstance(comparison, dict):
        return set()
    regressed = comparison.get("regressed")
    names: set[str] = set()
    if isinstance(regressed, list):
        for entry in regressed:
            if isinstance(entry, dict) and entry.get("task_name"):
                names.add(normalize_task_name(entry["task_name"]))
    return names


def miner_evidence_by_task(miner_json: Any) -> dict[str, dict[str, Any]]:
    """Index miner evidence records by their normalized task slug."""
    result: dict[str, dict[str, Any]] = {}
    if not isinstance(miner_json, list):
        return result
    for record in miner_json:
        if not isinstance(record, dict) or "task" not in record:
            continue
        result[normalize_task_name(record["task"])] = record
    return result


def _evidence_strings(record: dict[str, Any]) -> list[str]:
    raw = record.get("evidence")
    items: list[str] = []
    if isinstance(raw, list):
        for item in raw[:MAX_EVIDENCE_ITEMS]:
            if isinstance(item, str) and item.strip():
                items.append(_clip(item))
    return items


def build_failure_records(
    *,
    analysis: dict[str, Any],
    comparison: dict[str, Any] | None = None,
    miner_evidence: Any = None,
) -> list[FailureRecord]:
    fail_tasks = clean_run_fail_tasks(analysis)
    classes = task_classes_map(analysis)
    regressed = regressed_task_names(comparison)
    evidence = miner_evidence_by_task(miner_evidence)

    records: list[FailureRecord] = []
    for task in sorted(fail_tasks):
        analyzer_classes = sorted(classes.get(task, set()))
        miner = evidence.get(task)
        if miner is not None:
            route = str(miner.get("route") or route_from_classes(analyzer_classes))
            dominant = str(miner.get("dominant_cause") or "unknown")
            near_miss = bool(miner.get("near_miss", False))
            longer_window = bool(miner.get("longer_window_would_help", False))
            verifier_failure = miner.get("verifier_failure")
            evidence_strings = _evidence_strings(miner)
        else:
            route = route_from_classes(analyzer_classes)
            dominant = "unknown"
            near_miss = False
            longer_window = "deadline-extension" == route
            verifier_failure = None
            evidence_strings = []

        if route not in ROUTE_CLASSES:
            raise ValueError(f"task {task!r} has unknown route {route!r}")

        records.append(
            FailureRecord(
                task=task,
                analyzer_classes=analyzer_classes,
                dominant_cause=dominant,
                route=route,
                regression=task in regressed,
                near_miss=near_miss,
                longer_window_would_help=longer_window,
                task_timeout_sec=task_timeout(task),
                verifier_failure=_clip(verifier_failure) if isinstance(verifier_failure, str) else None,
                evidence=evidence_strings,
                has_miner_evidence=miner is not None,
            )
        )
    return records


def route_histogram(records: list[FailureRecord]) -> dict[str, int]:
    counts = {route: 0 for route in ROUTE_CLASSES}
    for record in records:
        counts[record.route] = counts.get(record.route, 0) + 1
    return {route: count for route, count in counts.items() if count}


def build_report(
    *,
    analysis: dict[str, Any],
    comparison: dict[str, Any] | None,
    miner_evidence: Any,
) -> dict[str, Any]:
    records = build_failure_records(
        analysis=analysis, comparison=comparison, miner_evidence=miner_evidence
    )
    return {
        "job_name": analysis.get("job_name"),
        "cleanRunSoftWindowSec": CLEAN_RUN_SOFT_WINDOW_SEC,
        "summary": {
            "failures": len(records),
            "regressions": sum(1 for r in records if r.regression),
            "nearMisses": sum(1 for r in records if r.near_miss),
            "withMinerEvidence": sum(1 for r in records if r.has_miner_evidence),
            "routeHistogram": route_histogram(records),
        },
        "records": [record.to_json() for record in records],
    }


def render_markdown(report: dict[str, Any]) -> str:
    summary = report.get("summary", {})
    lines = [
        "# TBench Failure Miner",
        "",
        f"- Job: `{report.get('job_name') or '<unknown>'}`",
        f"- Reward-0 failures: {summary.get('failures')}",
        f"- Regressions vs comparison: {summary.get('regressions')}",
        f"- Near misses: {summary.get('nearMisses')}",
        f"- Records with miner evidence: {summary.get('withMinerEvidence')}",
        "",
        "## Route histogram",
        "",
        "| Route | Tasks |",
        "| --- | ---: |",
    ]
    for route, count in sorted(summary.get("routeHistogram", {}).items()):
        lines.append(f"| `{route}` | {count} |")
    lines += [
        "",
        "## Failure records",
        "",
        "| Task | Route | Cause | Regression | Near-miss | Timeout (s) |",
        "| --- | --- | --- | :---: | :---: | ---: |",
    ]
    for record in report.get("records", []):
        lines.append(
            "| `{task}` | `{route}` | {cause} | {reg} | {nm} | {to} |".format(
                task=record.get("task"),
                route=record.get("route"),
                cause=record.get("dominantCause"),
                reg="yes" if record.get("regression") else "-",
                nm="yes" if record.get("nearMiss") else "-",
                to=record.get("taskTimeoutSec") if record.get("taskTimeoutSec") is not None else "?",
            )
        )
    return "\n".join(lines).rstrip() + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analysis", type=Path, required=True)
    parser.add_argument("--comparison", type=Path)
    parser.add_argument("--miner-evidence", type=Path)
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        analysis = load_json(args.analysis)
        comparison = load_json(args.comparison) if args.comparison else None
        miner_evidence = load_json(args.miner_evidence) if args.miner_evidence else None
        report = build_report(
            analysis=analysis, comparison=comparison, miner_evidence=miner_evidence
        )
    except Exception as exc:  # noqa: BLE001 - CLI boundary
        print(f"tbench_failure_miner: {exc}", file=sys.stderr)
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
