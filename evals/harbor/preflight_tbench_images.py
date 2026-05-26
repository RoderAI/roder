#!/usr/bin/env python3
"""Preflight Terminal-Bench Docker images for the Roder Harbor harness."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


DEFAULT_REGISTRY_URL = "https://raw.githubusercontent.com/laude-institute/harbor/main/registry.json"
TASK_CACHE_DIR = Path("~/.cache/harbor/tasks").expanduser()
TRANSIENT_REGISTRY_MARKERS = (
    "Bad Gateway",
    "502",
    "503",
    "504",
    "Service Unavailable",
    "Gateway Timeout",
)


@dataclass(frozen=True)
class TaskSpec:
    name: str
    git_url: str | None
    git_commit_id: str | None
    path: str


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def fetch_json(url: str) -> Any:
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.loads(response.read())


def read_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text())


def registry_url_for_dataset(dataset_config: dict[str, Any]) -> str:
    registry = dataset_config.get("registry")
    if isinstance(registry, dict):
        url = registry.get("url")
        if isinstance(url, str) and url:
            return url
    return DEFAULT_REGISTRY_URL


def load_registry_dataset(dataset_config: dict[str, Any]) -> list[TaskSpec]:
    registry_data = fetch_json(registry_url_for_dataset(dataset_config))
    name = dataset_config.get("name")
    version = dataset_config.get("version")
    for dataset in registry_data:
        if dataset.get("name") == name and dataset.get("version") == version:
            return [
                TaskSpec(
                    name=str(task["name"]),
                    git_url=task.get("git_url"),
                    git_commit_id=task.get("git_commit_id"),
                    path=str(task["path"]),
                )
                for task in dataset.get("tasks", [])
            ]
    raise ValueError(f"Dataset not found in registry: {name}@{version}")


def selected_tasks(
    config: dict[str, Any],
    allow_network: bool,
) -> tuple[list[TaskSpec], list[str]]:
    tasks: list[TaskSpec] = []
    unresolved: list[str] = []
    datasets = config.get("datasets") or []
    for dataset_config in datasets:
        if not isinstance(dataset_config, dict):
            continue
        task_names = dataset_config.get("task_names")
        if isinstance(task_names, list) and task_names:
            wanted = [str(name) for name in task_names]
        elif allow_network:
            wanted = []
        else:
            unresolved.append(
                f"{dataset_config.get('name')}@{dataset_config.get('version')}: "
                "offline mode needs explicit task_names for image preflight"
            )
            continue

        registry_tasks: list[TaskSpec] = []
        if allow_network:
            registry_tasks = load_registry_dataset(dataset_config)

        if wanted and registry_tasks:
            by_name = {task.name: task for task in registry_tasks}
            for name in wanted:
                if name in by_name:
                    tasks.append(by_name[name])
                else:
                    unresolved.append(f"{name}: not found in registry")
        elif wanted:
            for name in wanted:
                tasks.append(TaskSpec(name=name, git_url=None, git_commit_id=None, path=name))
        else:
            tasks.extend(registry_tasks)
    return tasks, unresolved


def cached_task_toml(task_name: str) -> Path | None:
    for path in TASK_CACHE_DIR.glob(f"*/{task_name}/task.toml"):
        return path
    return None


def raw_github_task_toml_url(task: TaskSpec) -> str | None:
    if not task.git_url or not task.git_commit_id:
        return None
    parsed = urllib.parse.urlparse(task.git_url)
    if parsed.netloc != "github.com":
        return None
    repo = parsed.path.strip("/")
    if repo.endswith(".git"):
        repo = repo[:-4]
    return (
        f"https://raw.githubusercontent.com/{repo}/{task.git_commit_id}/"
        f"{task.path.rstrip('/')}/task.toml"
    )


def fetch_task_toml(task: TaskSpec) -> dict[str, Any] | None:
    url = raw_github_task_toml_url(task)
    if url is None:
        return None
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            return tomllib.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, OSError, tomllib.TOMLDecodeError):
        return None


def resolve_task_image(task: TaskSpec, allow_network: bool) -> tuple[str | None, str]:
    cached = cached_task_toml(task.name)
    if cached:
        try:
            data = read_toml(cached)
            image = data.get("environment", {}).get("docker_image")
            if isinstance(image, str) and image:
                return image, f"cache:{cached}"
        except Exception:
            pass
    if allow_network:
        data = fetch_task_toml(task)
        if data:
            image = data.get("environment", {}).get("docker_image")
            if isinstance(image, str) and image:
                return image, "registry-source"
    return None, "unresolved"


def run_command(args: list[str], timeout: int | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )


def docker_image_id(image: str) -> str | None:
    result = run_command(["docker", "image", "inspect", image, "--format", "{{.Id}}"], timeout=30)
    if result.returncode == 0:
        return result.stdout.strip()
    return None


def pull_image(image: str, retries: int) -> tuple[bool, list[dict[str, Any]]]:
    attempts: list[dict[str, Any]] = []
    for attempt in range(1, retries + 1):
        result = run_command(["docker", "pull", image], timeout=None)
        text = f"{result.stdout}\n{result.stderr}".strip()
        attempts.append(
            {
                "attempt": attempt,
                "return_code": result.returncode,
                "stdout": result.stdout[-4000:],
                "stderr": result.stderr[-4000:],
                "transient_registry_failure": any(
                    marker in text for marker in TRANSIENT_REGISTRY_MARKERS
                ),
            }
        )
        if result.returncode == 0:
            return True, attempts
        if attempt < retries:
            time.sleep(min(60, 2**attempt))
    return False, attempts


def preflight(config_path: Path, offline: bool, pull: bool, retries: int) -> dict[str, Any]:
    if pull and os.environ.get("RODER_HARBOR_PREFLIGHT_PULL") != "1":
        raise RuntimeError("Set RODER_HARBOR_PREFLIGHT_PULL=1 to pull Docker images")

    config = load_json(config_path)
    allow_network = not offline or pull
    tasks, unresolved_selection = selected_tasks(config, allow_network=allow_network)
    entries: list[dict[str, Any]] = []
    images: dict[str, dict[str, Any]] = {}

    for task in tasks:
        image, source = resolve_task_image(task, allow_network=allow_network)
        entry: dict[str, Any] = {
            "task_name": task.name,
            "task_path": task.path,
            "image": image,
            "image_source": source,
        }
        if image is None:
            entry["status"] = "unresolved"
            entries.append(entry)
            continue

        image_id = docker_image_id(image)
        image_entry = images.setdefault(
            image,
            {
                "image": image,
                "tasks": [],
                "present_before": image_id is not None,
                "image_id_before": image_id,
                "status": "present" if image_id else "missing",
                "pull_attempts": [],
            },
        )
        image_entry["tasks"].append(task.name)
        if image_id:
            entry["status"] = "present"
            entry["image_id"] = image_id
        elif pull:
            ok, attempts = pull_image(image, retries=retries)
            image_entry["pull_attempts"] = attempts
            image_entry["status"] = "pulled" if ok else "pull_failed"
            image_id_after = docker_image_id(image)
            image_entry["image_id_after"] = image_id_after
            entry["status"] = "pulled" if ok else "pull_failed"
            if image_id_after:
                entry["image_id"] = image_id_after
        else:
            entry["status"] = "missing"
        entries.append(entry)

    summary = {
        "tasks": len(entries),
        "unique_images": len(images),
        "present": sum(1 for entry in entries if entry["status"] in {"present", "pulled"}),
        "missing": sum(1 for entry in entries if entry["status"] == "missing"),
        "unresolved": sum(1 for entry in entries if entry["status"] == "unresolved")
        + len(unresolved_selection),
        "pull_failed": sum(1 for entry in entries if entry["status"] == "pull_failed"),
    }
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "config": str(config_path),
        "offline": offline,
        "pull": pull,
        "summary": summary,
        "selection_errors": unresolved_selection,
        "tasks": entries,
        "images": sorted(images.values(), key=lambda item: item["image"]),
        "clean": not (
            summary["missing"]
            or summary["unresolved"]
            or summary["pull_failed"]
            or unresolved_selection
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--pull", action="store_true")
    parser.add_argument("--retries", type=int, default=4)
    parser.add_argument("--manifest", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        manifest = preflight(
            config_path=args.config,
            offline=args.offline,
            pull=args.pull,
            retries=max(1, args.retries),
        )
    except Exception as exc:
        print(f"preflight_tbench_images: {exc}", file=sys.stderr)
        return 2

    if args.manifest:
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(json.dumps(manifest, indent=2) + "\n")

    summary = manifest["summary"]
    print(
        "TBench image preflight: "
        f"tasks={summary['tasks']} unique_images={summary['unique_images']} "
        f"present={summary['present']} missing={summary['missing']} "
        f"unresolved={summary['unresolved']} pull_failed={summary['pull_failed']}"
    )
    if not manifest["clean"]:
        for error in manifest["selection_errors"]:
            print(f"selection error: {error}", file=sys.stderr)
        for entry in manifest["tasks"]:
            if entry["status"] in {"missing", "unresolved", "pull_failed"}:
                print(
                    f"{entry['status']}: {entry['task_name']} image={entry.get('image')}",
                    file=sys.stderr,
                )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
