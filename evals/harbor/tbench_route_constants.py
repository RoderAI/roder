#!/usr/bin/env python3
"""Canonical constants for Terminal-Bench score-route mining and campaigns.

Single source of truth shared by ``tbench_failure_miner`` and
``tbench_score_routes`` (and their tests). Keeping the reference data here
mirrors the ``tbench_diagnostic_contract`` / ``tbench_analysis_constants``
split so the two tools never disagree about routes, per-task windows, or
harbor invocation defaults.
"""

from __future__ import annotations

import re

# The window (seconds) the clean soft780 run left the agent. Used by the
# conservative deadline-extension conversion projection: a deadline-burn task
# only projects as a conversion when its Terminal-Bench default agent timeout
# exceeds this shortened window.
CLEAN_RUN_SOFT_WINDOW_SEC = 780

# Canonical route classes. The first seven are defined by PRD Stage 4; the
# eighth, ``capability``, is the no-harness-fix bucket (guidance-level only).
ROUTE_CLASSES: tuple[str, ...] = (
    "deadline-extension",
    "task-contract",
    "environment-service",
    "policy-framed",
    "historical-regression",
    "plan-first",
    "codex-parity",
    "capability",
)

# Routes whose tasks can be launched as targeted reruns right now.
RUNNABLE_ROUTES: frozenset[str] = frozenset(
    {
        "deadline-extension",
        "task-contract",
        "policy-framed",
        "historical-regression",
        "plan-first",
        "codex-parity",
    }
)

# Routes that are real reruns but cannot run on the current host. QEMU / torch
# tasks need a native x86_64 Linux docker host (Rosetta lacks signalfd/282).
BLOCKED_ROUTES: frozenset[str] = frozenset({"environment-service"})

# Routes surfaced in the summary only, never in a runnable campaign manifest.
SUMMARY_ONLY_ROUTES: frozenset[str] = frozenset({"capability"})

ROUTE_DESCRIPTIONS: dict[str, str] = {
    "deadline-extension": (
        "Deadline-burn regressions: the shortened soft780/eval720 internal ladder "
        "cut the agent off below the task's Terminal-Bench default window. Fix is a "
        "per-task window derived from the task's real agent timeout."
    ),
    "task-contract": (
        "Near-miss lost on the scored artifact's call/interface contract, not on time. "
        "Fix is a fresh-process full-input smoke test before finalizing."
    ),
    "environment-service": (
        "Environment/service gap on the Apple-Silicon/Rosetta host (signalfd 282, "
        "python-runtime discovery). Needs a native x86_64 Linux docker host."
    ),
    "policy-framed": (
        "Stochastic provider cyber-policy block on turn 1 before any work. Fix is a "
        "bounded fresh-thread retry (implemented concurrently in the adapter)."
    ),
    "historical-regression": (
        "Passed in the dirty run but the clean run finalized on a false completion "
        "(self-verification gate too weak). Fix is requirement-to-evidence validation."
    ),
    "plan-first": "Reserved: failures a plan-first phase would convert (no evidence-backed tasks yet).",
    "codex-parity": "Reserved: Roder-specific overhead A/B experiments (no evidence-backed tasks yet).",
    "capability": (
        "No harness fix identified; the binding constraint is model capability. "
        "Candidates for guidance-level improvements only, not runnable campaigns."
    ),
}

BLOCKED_HOST_NOTE = (
    "blocked-on-host: requires a native x86_64 Linux docker host. Rosetta/OrbStack on "
    "Apple Silicon lacks syscall 282 (signalfd), which crashes qemu-system-x86_64 and "
    "hides the uv-managed torch runtime. Not runnable on the current host."
)

CAPABILITY_NOTE = (
    "No harness fix identified; capability-bound. The dirty long-window run also failed "
    "(or false-completed), so a window/finalization lever will not convert these. "
    "Candidates for guidance-level improvements only, NOT a runnable campaign manifest."
)

# Authoritative per-task Terminal-Bench agent timeouts (seconds), from the
# dataset task.toml cache. These are the full default windows a leaderboard-valid
# run would grant each task.
TASK_TIMEOUTS: dict[str, int] = {
    "break-filter-js-from-html": 1200,
    "circuit-fibsqrt": 3600,
    "dna-assembly": 1800,
    "feal-linear-cryptanalysis": 1800,
    "make-doom-for-mips": 900,
    "make-mips-interpreter": 1800,
    "protein-assembly": 1800,
    "regex-chess": 3600,
    "sam-cell-seg": 7200,
    "train-fasttext": 3600,
    "compile-compcert": 2400,
    "crack-7z-hash": 900,
    "extract-moves-from-video": 1800,
    "gcode-to-text": 900,
    "gpt2-codegolf": 900,
    "install-windows-3.11": 3600,
    "mailman": 1800,
    "qemu-alpine-ssh": 900,
    "winning-avg-corewars": 3600,
    "model-extraction-relu-logits": 900,
    "build-cython-ext": 900,
    "caffe-cifar-10": 1200,
    "chess-best-move": 900,
    "db-wal-recovery": 900,
    "dna-insert": 1800,
    "filter-js-from-html": 1800,
    "headless-terminal": 900,
    "mcmc-sampling-stan": 1800,
    "path-tracing": 1800,
    "path-tracing-reverse": 1800,
    "pytorch-model-recovery": 900,
    "qemu-startup": 900,
    "raman-fitting": 900,
    "rstan-to-pystan": 1800,
    "torch-pipeline-parallelism": 900,
    "video-processing": 3600,
}

# Harbor invocation defaults for a targeted rerun.
HARBOR_DATASET = "terminal-bench/terminal-bench-2-1"
HARBOR_AGENT = "roder_harbor_agent:RoderCli"
HARBOR_MODEL = "codex/gpt-5.5"
HARBOR_JOBS_DIR = "evals/harbor/jobs"

# Default per-route ``--ak`` kwargs. reasoning=xhigh matches every mined trial.
# Route knobs beyond reasoning are the real adapter kwargs from
# roder_harbor_agent_settings so a manifest's harbor invocation runs verbatim:
# the deadline-extension route enables the per-task deadline ladder (each task's
# own Terminal-Bench window at multiplier 1.0), and the policy-framed route sets
# the bounded zero-progress policy-block retry.
ROUTE_AK: dict[str, dict[str, str]] = {
    "deadline-extension": {
        "reasoning": "xhigh",
        "per_task_deadlines": "true",
        "agent_timeout_multiplier_hint": "1.0",
    },
    "task-contract": {"reasoning": "xhigh"},
    "policy-framed": {"reasoning": "xhigh", "policy_block_max_retries": "2"},
    "historical-regression": {"reasoning": "xhigh"},
    "environment-service": {"reasoning": "xhigh"},
    "plan-first": {"reasoning": "xhigh"},
    "codex-parity": {"reasoning": "xhigh"},
}

# Track labels for manifests.
TRACK_LEADERBOARD = "leaderboard-valid-candidate"
TRACK_LOCAL_ONLY = "local-only"

_TASK_PREFIX = "terminal-bench/"
_NAME_SPLIT = re.compile(r"[\s:(]")


def normalize_task_name(name: str) -> str:
    """Reduce an analyzer / miner task label to its bare Terminal-Bench slug.

    Handles the ``terminal-bench/`` analyzer prefix and the long descriptive
    suffixes miner evidence carries (e.g. ``"sam-cell-seg: implement ..."`` or
    ``"dna-assembly (Terminal-Bench 2.1) - ..."``).
    """
    text = str(name).strip()
    if text.startswith(_TASK_PREFIX):
        text = text[len(_TASK_PREFIX) :]
    # Strip a Harbor trial-hash suffix ("crack-7z-hash__c1") before the descriptive tail.
    text = text.split("__", 1)[0]
    token = _NAME_SPLIT.split(text, maxsplit=1)[0]
    return token.strip().strip(":,")


def route_from_classes(classes: set[str] | frozenset[str] | list[str]) -> str:
    """Fallback route assignment from analyzer classes when no miner evidence exists."""
    present = set(classes)
    if "provider_policy_block" in present:
        return "policy-framed"
    if present.intersection(
        {"internal_deadline_timeout", "soft_timeout", "soft_timeout_fail", "deadline_finalized"}
    ):
        return "deadline-extension"
    return "capability"


def task_timeout(task_name: str) -> int | None:
    return TASK_TIMEOUTS.get(normalize_task_name(task_name))
