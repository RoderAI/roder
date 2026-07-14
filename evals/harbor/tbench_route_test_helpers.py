#!/usr/bin/env python3
"""Shared synthetic fixtures for the failure-miner and score-routes tests.

Not a test module. Builds analyzer-shaped, comparison-shaped, and miner-evidence-shaped
dicts covering one task per route so both test files can assert route assignment,
projection, guards, and track labelling without touching real job dirs.
"""

from __future__ import annotations

from typing import Any

EVAL_KEY = "roder-cli__gpt-5.5__terminal-bench/terminal-bench-2-1"

# One reward-0 task per route class we exercise. (task, trial_hash, extra analyzer classes)
FAIL_TASKS: list[tuple[str, str, list[str]]] = [
    ("crack-7z-hash", "c1", ["internal_deadline_timeout", "soft_timeout", "soft_timeout_fail"]),
    ("regex-chess", "d2", ["deadline_finalized"]),
    ("pytorch-model-recovery", "e3", []),
    ("model-extraction-relu-logits", "f4", ["provider_policy_block"]),
    ("headless-terminal", "g5", []),
    ("qemu-startup", "h6", []),
    ("torch-pipeline-parallelism", "i7", []),
    ("make-doom-for-mips", "j8", []),
]

PASS_TASKS: list[tuple[str, str]] = [("extract-elf", "p1"), ("fix-git", "p2")]

# Regressed tasks (dirty pass -> clean fail); a subset of the reward-0 set.
REGRESSED = ["crack-7z-hash", "model-extraction-relu-logits", "headless-terminal"]

# Miner evidence: route is ground truth; near_miss drives the conversion projection.
MINER_RECORDS: list[dict[str, Any]] = [
    {
        "task": "crack-7z-hash: recover the 7z archive password",  # descriptive suffix on purpose
        "dominant_cause": "deadline_burn",
        "route": "deadline-extension",
        "near_miss": True,
        "longer_window_would_help": True,
        "harness_fix": "Restore the full Terminal-Bench default agent window; token: sk-ABCDEF1234567890 leaked here.",
        "verifier_failure": "assert 'password' == 'honeybear'",
        "evidence": ["provisional 'password' written to /app/solution.txt", "dirty run passed at 1261s"],
    },
    {
        "task": "regex-chess",
        "dominant_cause": "deadline_burn",
        "route": "deadline-extension",
        "near_miss": False,
        "longer_window_would_help": True,
        "harness_fix": "Derive the eval-deadline ladder from the task's real agent timeout.",
        "verifier_failure": "3 failed, 1 passed",
        "evidence": ["re.json echoed the input FEN"],
    },
    {
        "task": "pytorch-model-recovery",
        "dominant_cause": "task_contract_miss",
        "route": "task-contract",
        "near_miss": True,
        "longer_window_would_help": False,
        "harness_fix": "Smoke-test the reloaded artifact with all provided inputs before finalizing.",
        "verifier_failure": "forward() expected at most 2 arguments but received 3",
        "evidence": ["agent scripted single-arg forward"],
    },
    {
        "task": "model-extraction-relu-logits",
        "dominant_cause": "policy_block",
        "route": "policy-framed",
        "near_miss": False,
        "longer_window_would_help": False,
        "harness_fix": "Bounded fresh-thread retry on a first-turn policy_block.",
        "verifier_failure": "File /app/stolen_A1.npy does not exist",
        "evidence": ["first model turn refused after 4s"],
    },
    {
        "task": "headless-terminal",
        "dominant_cause": "false_completion",
        "route": "historical-regression",
        "near_miss": True,
        "longer_window_would_help": False,
        "harness_fix": "Require requirement-to-evidence mapping in the verification gate.",
        "verifier_failure": "Screen.select_graphic_rendition() got an unexpected keyword argument 'private'",
        "evidence": ["verification_review reported openGaps: [] with no interactive program test"],
    },
    {
        "task": "qemu-startup",
        "dominant_cause": "env_service_gap",
        "route": "environment-service",
        "near_miss": False,
        "longer_window_would_help": False,
        "harness_fix": "Run amd64 task containers on a native x86_64 Linux runner (Rosetta lacks signalfd 282).",
        "verifier_failure": "FileNotFoundError: /tmp/data.txt",
        "evidence": ["rosetta error: Unimplemented syscall number 282"],
    },
    {
        "task": "torch-pipeline-parallelism",
        "dominant_cause": "env_service_gap",
        "route": "environment-service",
        "near_miss": True,
        "longer_window_would_help": False,
        "harness_fix": "Add a runtime-inventory probe; a uv-cached torch runtime was present.",
        "verifier_failure": "Rank 0 mismatch at lm_head.bwd",
        "evidence": ["python3 had no torch; verifier imported torch from uv cache"],
    },
    {
        "task": "make-doom-for-mips",
        "dominant_cause": "capability_gap",
        "route": "capability",
        "near_miss": False,
        "longer_window_would_help": False,
        "harness_fix": "Guidance: apt install a MIPS cross-compiler is an expected first resort.",
        "verifier_failure": "File /tmp/frame.bmp does not exist",
        "evidence": ["never attempted apt-get install of a cross compiler"],
    },
]


def analysis() -> dict[str, Any]:
    classes: dict[str, list[dict[str, Any]]] = {"pass": [], "scored_fail": []}
    for task, trial_hash in PASS_TASKS:
        classes["pass"].append(
            {"trial_name": f"{task}__{trial_hash}", "task_name": f"terminal-bench/{task}", "reward": 1.0}
        )
    for task, trial_hash, extra in FAIL_TASKS:
        entry = {
            "trial_name": f"{task}__{trial_hash}",
            "task_name": f"terminal-bench/{task}",
            "reward": 0.0,
        }
        classes["scored_fail"].append(entry)
        for cls in extra:
            classes.setdefault(cls, []).append(dict(entry))
    reward = {
        "1.0": [f"{task}__{h}" for task, h in PASS_TASKS],
        "0.0": [f"{task}__{h}" for task, h, _ in FAIL_TASKS],
    }
    return {
        "job_name": "synthetic-clean-run",
        "stats": {
            "harbor": {"evals": {EVAL_KEY: {"reward_stats": {"reward": reward}}}},
            "passes": len(PASS_TASKS),
            "scored_failures": len(FAIL_TASKS),
        },
        "classes": classes,
    }


def comparison() -> dict[str, Any]:
    return {
        "regressed": [
            {
                "task_name": f"terminal-bench/{task}",
                "baseline": {"reward": 1.0, "classes": ["pass"]},
                "current": {"reward": 0.0, "classes": ["scored_fail"]},
            }
            for task in REGRESSED
        ],
        "improved": [],
    }


def miner_evidence() -> list[dict[str, Any]]:
    return [dict(record) for record in MINER_RECORDS]
