#!/usr/bin/env python3
"""Create missing baseline release tags (<package>/v<version>) for knope.

knope's Release step publishes a GitHub release for every package whose
version has no matching tag. Without baseline tags, the first release after
adopting knope would publish all ~90 packages at once. Run this once from
master after merging the knope setup (and after adding a new package, if you
don't want its unchanged version released):

    python3 scripts/init-release-tags.py         # create tags locally
    python3 scripts/init-release-tags.py --push  # ...and push them
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        # Auto-install tomli for Python < 3.11 (e.g. system python3 on older macOS).
        # This keeps the release scripts runnable with plain `python3` without
        # requiring a prior `pip install tomli` or mise/uv env.
        subprocess = __import__("subprocess")
        subprocess.check_call([sys.executable, "-m", "pip", "install", "--user", "tomli"])
        import tomli as tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def read_version(versioned_file: Path) -> str:
    if versioned_file.name == "Cargo.toml":
        with versioned_file.open("rb") as f:
            return tomllib.load(f)["package"]["version"]
    if versioned_file.name == "package.json":
        return json.loads(versioned_file.read_text())["version"]
    if versioned_file.name == "pyproject.toml":
        with versioned_file.open("rb") as f:
            return tomllib.load(f)["project"]["version"]
    raise ValueError(f"unsupported versioned file: {versioned_file}")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--push", action="store_true", help="push created tags to origin")
    args = parser.parse_args()

    with (REPO_ROOT / "knope.toml").open("rb") as f:
        config = tomllib.load(f)

    existing = set(git("tag", "--list").splitlines())
    created = []
    for name, package in config.get("packages", {}).items():
        first = package["versioned_files"][0]
        path = first["path"] if isinstance(first, dict) else first
        version = read_version(REPO_ROOT / path)
        tag = f"{name}/v{version}"
        if tag in existing:
            continue
        git("tag", tag)
        created.append(tag)
        print(f"created {tag}")

    if not created:
        print("All baseline tags already exist.")
        return 0
    if args.push:
        subprocess.run(["git", "push", "origin", *created], cwd=REPO_ROOT, check=True)
        print(f"Pushed {len(created)} tags.")
    else:
        print(f"Created {len(created)} tags locally. Push them with --push.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
