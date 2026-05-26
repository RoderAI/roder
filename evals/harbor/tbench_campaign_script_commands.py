"""Command parsing helpers for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import re
import shlex
from typing import Any


REQUIRED_BASELINE_PATH = "evals/harbor/tbench-clean-baseline.json"


def validate_route_command_order(result: Any, tokens: list[str], routes: list[Any]) -> None:
    bad_routes: list[str] = []
    for route in routes:
        if not isinstance(route, dict):
            continue
        config = route.get("config")
        job_dir = route.get("jobDir")
        analysis_json = route.get("analysisJson")
        if not all(isinstance(value, str) and value for value in (config, job_dir, analysis_json)):
            continue
        positions = (
            harbor_run_index(tokens, str(config)),
            analysis_command_index(tokens, str(job_dir)),
            baseline_validation_command_index(tokens, str(analysis_json)),
        )
        if None not in positions and not (positions[0] < positions[1] < positions[2]):
            bad_routes.append(str(route.get("name") or "<missing>"))
    if bad_routes:
        result.add("runScript route command order mismatch: " + ", ".join(bad_routes))


def validate_final_campaign_validation_order(
    result: Any,
    tokens: list[str],
    routes: list[Any],
) -> None:
    final_index = final_campaign_validation_index(tokens)
    baseline_indexes = [
        baseline_validation_command_index(tokens, str(route.get("analysisJson")))
        for route in routes
        if isinstance(route, dict) and isinstance(route.get("analysisJson"), str)
    ]
    baseline_indexes = [index for index in baseline_indexes if index is not None]
    if final_index is not None and baseline_indexes and final_index <= max(baseline_indexes):
        result.add("runScript final campaign validation order mismatch")


def harbor_run_index(tokens: list[str], config: str) -> int | None:
    for index, token in enumerate(tokens):
        if (
            token == "harbor"
            and index + 3 < len(tokens)
            and tokens[index + 1] == "run"
            and tokens[index + 2] == "--config"
            and tokens[index + 3] == config
        ):
            return index
    return None


def analysis_command_index(tokens: list[str], job_dir: str) -> int | None:
    for index, token in enumerate(tokens):
        if (
            token == "evals/harbor/analyze_tbench_run.py"
            and index + 1 < len(tokens)
            and tokens[index + 1] == job_dir
        ):
            return index
    return None


def baseline_validation_command_index(tokens: list[str], analysis_json: str) -> int | None:
    for index, token in enumerate(tokens):
        if (
            token == "evals/harbor/validate_tbench_analysis.py"
            and index + 1 < len(tokens)
            and tokens[index + 1] == analysis_json
        ):
            return index
    return None


def final_campaign_validation_index(tokens: list[str]) -> int | None:
    for index, token in enumerate(tokens):
        if token != "evals/harbor/validate_tbench_campaign.py":
            continue
        window = tokens[index + 2 : index + 10]
        if "--require-image-preflight" in window and "--require-analysis" in window:
            return index
    return None


def expected_route_job_dirs(routes: list[Any]) -> list[str]:
    return [
        str(route.get("jobDir"))
        for route in routes
        if isinstance(route, dict) and isinstance(route.get("jobDir"), str)
    ]


def route_job_dir_values(text: str) -> list[str]:
    match = re.search(r"^route_job_dirs=\(\n(?P<body>.*?)^\)", text, re.MULTILINE | re.DOTALL)
    if not match:
        return []
    return [value for value in shlex.split(match.group("body")) if value]


def array_literal_values(text: str, array_name: str) -> list[str]:
    multiline = re.compile(
        r"^\s*"
        + re.escape(array_name)
        + r"=\(\n(?P<body>.*?)^\s*\)",
        re.MULTILINE | re.DOTALL,
    )
    match = multiline.search(text)
    if not match:
        single_line = re.compile(
            r"^\s*"
            + re.escape(array_name)
            + r"=\((?P<body>[^\n]*)\)\s*$",
            re.MULTILINE,
        )
        match = single_line.search(text)
    if not match:
        return []
    return [value for value in shlex.split(match.group("body"), comments=True) if value]


def command_flag_values(
    tokens: list[str],
    command: str,
    subcommand: str,
    flag: str,
) -> list[str]:
    values: list[str] = []
    for index, token in enumerate(tokens):
        if (
            token == command
            and index + 3 < len(tokens)
            and tokens[index + 1] == subcommand
            and tokens[index + 2] == flag
        ):
            values.append(tokens[index + 3])
    return values


def script_flag_values(tokens: list[str], script: str, flag: str) -> list[str]:
    values: list[str] = []
    for index, token in enumerate(tokens):
        if token == script and index + 2 < len(tokens) and tokens[index + 1] == flag:
            values.append(tokens[index + 2])
    return values


def expected_image_preflight_tuples(routes: list[Any]) -> list[tuple[str, str, str]]:
    tuples: list[tuple[str, str, str]] = []
    for route in routes:
        if not isinstance(route, dict):
            continue
        config = route.get("config")
        image_manifest = route.get("imageManifest")
        if isinstance(config, str) and config and isinstance(image_manifest, str) and image_manifest:
            tuples.append((config, "${preflight_args[@]}", image_manifest))
    return tuples


def image_preflight_command_tuples(tokens: list[str]) -> list[tuple[str, str, str]]:
    commands: list[tuple[str, str, str]] = []
    for index, token in enumerate(tokens):
        if token != "evals/harbor/preflight_tbench_images.py":
            continue
        window = tokens[index + 1 : index + 12]
        config = flag_value(window, "--config")
        manifest = flag_value(window, "--manifest")
        mode = preflight_mode_value(window)
        if config and mode and manifest:
            commands.append((config, mode, manifest))
    return commands


def preflight_mode_value(tokens: list[str]) -> str | None:
    for token in tokens:
        if token == "${preflight_args[@]}" or token in {"--offline", "--pull"}:
            return token
    return None


def expected_analysis_tuples(routes: list[Any]) -> list[tuple[str, str, str, str, str]]:
    tuples: list[tuple[str, str, str, str, str]] = []
    for route in routes:
        if not isinstance(route, dict):
            continue
        value = analysis_tuple(route)
        if value is not None:
            tuples.append(value)
    return tuples


def analysis_tuple(route: dict[str, Any]) -> tuple[str, str, str, str, str] | None:
    values = [
        route.get("jobDir"),
        route.get("analysisJson"),
        route.get("analysisMarkdown"),
        route.get("analysisManifestDir"),
    ]
    if not all(isinstance(value, str) and value for value in values):
        return None
    return (
        str(values[0]),
        "1",
        str(values[1]),
        str(values[2]),
        str(values[3]),
    )


def analysis_command_tuples(tokens: list[str]) -> list[tuple[str, str, str, str, str]]:
    commands: list[tuple[str, str, str, str, str]] = []
    for index, token in enumerate(tokens):
        if token != "evals/harbor/analyze_tbench_run.py" or index + 1 >= len(tokens):
            continue
        window = tokens[index + 2 : index + 16]
        values = {
            "--json": flag_value(window, "--json"),
            "--markdown": flag_value(window, "--markdown"),
            "--manifest-dir": flag_value(window, "--manifest-dir"),
        }
        if all(values.values()):
            commands.append(
                (
                    tokens[index + 1],
                    "1" if "--require-clean" in window else "0",
                    str(values["--json"]),
                    str(values["--markdown"]),
                    str(values["--manifest-dir"]),
                )
            )
    return commands


def expected_baseline_validation_tuples(routes: list[Any]) -> list[tuple[str, str, str]]:
    tuples: list[tuple[str, str, str]] = []
    for route in routes:
        if not isinstance(route, dict):
            continue
        analysis_json = route.get("analysisJson")
        task_count = route.get("taskCount")
        if isinstance(analysis_json, str) and analysis_json:
            count = int_value(task_count)
            if count:
                tuples.append((analysis_json, REQUIRED_BASELINE_PATH, str(count)))
    return tuples


def baseline_validation_command_tuples(tokens: list[str]) -> list[tuple[str, str, str]]:
    commands: list[tuple[str, str, str]] = []
    for index, token in enumerate(tokens):
        if token != "evals/harbor/validate_tbench_analysis.py" or index + 1 >= len(tokens):
            continue
        window = tokens[index + 2 : index + 12]
        baseline = flag_value(window, "--baseline")
        expected_trials = flag_value(window, "--expected-trials")
        if baseline and expected_trials:
            commands.append((tokens[index + 1], baseline, expected_trials))
    return commands


def expected_campaign_validation_tuples() -> list[tuple[str, str, str, str]]:
    return [
        ("$MANIFEST", "0", "0", ""),
        ("$MANIFEST", "1", "0", "$PREFLIGHT_DIR"),
        ("$MANIFEST", "1", "1", "$PREFLIGHT_DIR"),
    ]


def campaign_validation_command_tuples(tokens: list[str]) -> list[tuple[str, str, str, str]]:
    commands: list[tuple[str, str, str, str]] = []
    for index, token in enumerate(tokens):
        if token != "evals/harbor/validate_tbench_campaign.py" or index + 1 >= len(tokens):
            continue
        window = tokens[index + 2 : index + 10]
        require_image_preflight = "1" if "--require-image-preflight" in window else "0"
        require_analysis = "1" if "--require-analysis" in window else "0"
        commands.append(
            (
                tokens[index + 1],
                require_image_preflight,
                require_analysis,
                flag_value(window, "--preflight-dir") or "",
            )
        )
    return commands


def has_flag_value(tokens: list[str], flag: str, value: str) -> bool:
    return any(
        token == flag and index + 1 < len(tokens) and tokens[index + 1] == value
        for index, token in enumerate(tokens)
    )


def flag_value(tokens: list[str], flag: str) -> str | None:
    for index, token in enumerate(tokens):
        if token == flag and index + 1 < len(tokens):
            return tokens[index + 1]
    return None


def format_tuple(item: tuple[str, ...]) -> str:
    return ",".join(item)


def array_append_flag_values(text: str, array_name: str, flag: str) -> list[str]:
    pattern = re.compile(
        r"^\s*"
        + re.escape(array_name)
        + r"\+=\("
        + re.escape(flag)
        + r"\s+(.+?)\)\s*$",
        re.MULTILINE,
    )
    values: list[str] = []
    for match in pattern.finditer(text):
        parts = shlex.split(match.group(1))
        if parts:
            values.append(parts[0])
    return values


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
