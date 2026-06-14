#!/usr/bin/env python3
"""Verify registry packages have README metadata before publishing."""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

NPM_PACKAGES = [
    REPO_ROOT / "sdk/typescript/package.json",
    REPO_ROOT / "packages/edit-tools/package.json",
]
PYPI_PACKAGES = [
    REPO_ROOT / "sdk/python/pyproject.toml",
]
RODER_SITE = "https://roder.sh"


def fail(message: str) -> None:
    print(f"registry-readmes: {message}", file=sys.stderr)
    raise SystemExit(1)


def cargo_metadata() -> dict:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO_ROOT,
        text=True,
    )
    return json.loads(output)


def check_cargo_readmes() -> None:
    metadata = cargo_metadata()
    workspace_members = set(metadata["workspace_members"])
    missing = []
    missing_files = []
    non_local = []
    missing_site_link = []

    for package in metadata["packages"]:
        if package["id"] not in workspace_members:
            continue
        readme = package.get("readme")
        if not readme:
            missing.append(package["name"])
            continue
        readme_path = Path(readme)
        if not readme_path.is_absolute():
            manifest_dir = Path(package["manifest_path"]).parent
            readme_path = manifest_dir / readme_path
        expected_readme = Path(package["manifest_path"]).parent / "README.md"
        if readme_path != expected_readme:
            non_local.append(f"{package['name']}: {readme_path}")
        if not readme_path.exists():
            missing_files.append(f"{package['name']}: {readme_path}")
            continue
        if RODER_SITE not in readme_path.read_text():
            missing_site_link.append(package["name"])

    if missing:
        fail("Cargo packages missing readme metadata: " + ", ".join(sorted(missing)))
    if missing_files:
        fail("Cargo package readme files do not exist: " + "; ".join(sorted(missing_files)))
    if non_local:
        fail("Cargo packages must use package-local README.md files: " + "; ".join(sorted(non_local)))
    if missing_site_link:
        fail("Cargo package READMEs missing https://roder.sh: " + ", ".join(sorted(missing_site_link)))


def check_npm_readmes() -> None:
    for package_json in NPM_PACKAGES:
        data = json.loads(package_json.read_text())
        package_dir = package_json.parent
        readme = package_dir / "README.md"
        if not readme.exists():
            fail(f"{package_json.relative_to(REPO_ROOT)} has no README.md")
        if RODER_SITE not in readme.read_text():
            fail(f"{readme.relative_to(REPO_ROOT)} is missing {RODER_SITE}")
        files = data.get("files", [])
        if "README.md" not in files:
            fail(f"{package_json.relative_to(REPO_ROOT)} does not include README.md in files")


def check_pypi_readmes() -> None:
    for pyproject in PYPI_PACKAGES:
        with pyproject.open("rb") as handle:
            data = tomllib.load(handle)
        project = data.get("project", {})
        readme = project.get("readme")
        if not readme:
            fail(f"{pyproject.relative_to(REPO_ROOT)} has no project.readme")
        if isinstance(readme, str):
            readme_path = pyproject.parent / readme
            if not readme_path.exists():
                fail(f"{pyproject.relative_to(REPO_ROOT)} readme does not exist: {readme}")
            if RODER_SITE not in readme_path.read_text():
                fail(f"{readme_path.relative_to(REPO_ROOT)} is missing {RODER_SITE}")


def main() -> int:
    check_cargo_readmes()
    check_npm_readmes()
    check_pypi_readmes()
    print("registry-readmes: Cargo, npm, and PyPI package READMEs are present and link https://roder.sh.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
