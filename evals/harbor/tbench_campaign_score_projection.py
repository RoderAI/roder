"""Score projection helpers for generated Terminal-Bench campaigns."""

from __future__ import annotations

from math import ceil
from typing import Iterable

SUITE_TASKS = 89
BASELINE_PASSES = 50
CODEX_CLI_TARGET_SCORE = 0.820
SOTA_TARGET_SCORE = 0.847


def score_projection_for_tasks(task_names: Iterable[str]) -> dict[str, int | float]:
    candidates = len(set(task_names))
    projected_passes = min(SUITE_TASKS, BASELINE_PASSES + candidates)
    codex_target = ceil(CODEX_CLI_TARGET_SCORE * SUITE_TASKS)
    sota_target = ceil(SOTA_TARGET_SCORE * SUITE_TASKS)
    return {
        "suiteTasks": SUITE_TASKS,
        "baselinePasses": BASELINE_PASSES,
        "campaignConversionCandidates": candidates,
        "projectedPassesIfAllRoutesPass": projected_passes,
        "projectedMeanIfAllRoutesPass": projected_passes / SUITE_TASKS,
        "codexCliTargetPasses": codex_target,
        "codexCliGap": max(0, codex_target - projected_passes),
        "sotaTargetPasses": sota_target,
        "sotaGap": max(0, sota_target - projected_passes),
    }
