#!/usr/bin/env python3
"""Publish workspace crates to crates.io in dependency order."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import time
from collections import defaultdict, deque
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def is_publishable_to_crates_io(package: dict[str, object]) -> bool:
    registries = package.get("publish")
    return registries is None or "crates-io" in registries


def publish_order_from_metadata(meta: dict[str, object]) -> list[str]:
    members = {
        package["id"]: package
        for package in meta["packages"]
        if package["id"] in meta["workspace_members"]
        and is_publishable_to_crates_io(package)
    }
    name_to_id = {package["name"]: package_id for package_id, package in members.items()}
    graph: dict[str, set[str]] = defaultdict(set)
    indeg: dict[str, int] = defaultdict(int)

    for package_id, package in members.items():
        indeg.setdefault(package_id, 0)
        for dep in package.get("dependencies", []):
            dep_id = name_to_id.get(dep["name"])
            if dep_id and dep_id != package_id:
                dependents = graph[dep_id]
                if package_id not in dependents:
                    dependents.add(package_id)
                    indeg[package_id] += 1

    queue = deque(
        sorted(
            [package_id for package_id in members if indeg[package_id] == 0],
            key=lambda package_id: members[package_id]["name"],
        )
    )
    order: list[str] = []
    while queue:
        package_id = queue.popleft()
        order.append(members[package_id]["name"])
        for nxt in sorted(graph[package_id], key=lambda pid: members[pid]["name"]):
            indeg[nxt] -= 1
            if indeg[nxt] == 0:
                queue.append(nxt)

    if len(order) != len(members):
        omitted = sorted(
            members[package_id]["name"]
            for package_id in members
            if members[package_id]["name"] not in order
        )
        raise RuntimeError(
            "publish order omitted publishable workspace members "
            f"(dependency cycle or invalid graph): {', '.join(omitted)}"
        )

    return order


def publish_order() -> list[str]:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=REPO_ROOT,
            text=True,
        )
    )
    return publish_order_from_metadata(meta)


def latest_published_version(name: str) -> str | None:
    try:
        output = subprocess.check_output(
            ["cargo", "search", name, "--limit", "1"],
            cwd=REPO_ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return None
    first_line = output.splitlines()[0] if output else ""
    if not first_line.startswith(f"{name} ="):
        return None
    return first_line.split('"')[1]


def local_version(name: str) -> str:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=REPO_ROOT,
            text=True,
        )
    )
    for package in meta["packages"]:
        if package["name"] == name:
            return package["version"]
    raise KeyError(name)


def needs_publish(name: str) -> bool:
    published = latest_published_version(name)
    if published is None:
        return True
    return local_version(name) != published


def publish_package(name: str) -> None:
    while True:
        result = subprocess.run(
            ["cargo", "publish", "-p", name, "--allow-dirty"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            print(result.stdout, end="", flush=True)
            return

        combined = f"{result.stdout}\n{result.stderr}"
        print(combined, end="", flush=True)
        match = re.search(
            r"Please try again after ([A-Za-z]{3}, \d{2} [A-Za-z]{3} \d{4} \d{2}:\d{2}:\d{2} GMT)",
            combined,
        )
        if match:
            retry_at = datetime.strptime(match.group(1), "%a, %d %b %Y %H:%M:%S GMT").replace(
                tzinfo=timezone.utc
            )
            wait_seconds = max(1, int((retry_at - datetime.now(timezone.utc)).total_seconds()) + 1)
            print(
                f"publish-crates: rate limited; sleeping {wait_seconds}s before retrying {name}",
                flush=True,
            )
            time.sleep(wait_seconds)
            continue

        raise subprocess.CalledProcessError(
            result.returncode, result.args, result.stdout, result.stderr
        )


def main() -> int:
    published = 0
    skipped = 0

    for package in publish_order():
        print(f"publish-crates: {package}", flush=True)
        if not needs_publish(package):
            print(
                f"publish-crates: {package} already at {latest_published_version(package)} on crates.io; skipping",
                flush=True,
            )
            skipped += 1
            continue

        publish_package(package)
        published += 1
        time.sleep(2)

    print(f"publish-crates: done ({published} published, {skipped} skipped)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
