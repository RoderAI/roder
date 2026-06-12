#!/usr/bin/env python3
"""Generate knope.toml from the Cargo workspace members and SDK packages.

knope (https://knope.tech) drives changeset-based, per-package versioning and
releases for this repo. Every Cargo workspace crate plus the non-Rust packages
listed in EXTRA_PACKAGES gets its own [packages.<name>] entry.

Usage:
    python3 scripts/generate-knope-config.py          # rewrite knope.toml
    python3 scripts/generate-knope-config.py --check  # exit 1 if stale
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
KNOPE_TOML = REPO_ROOT / "knope.toml"
CRATES_DIR = REPO_ROOT / "crates"

# Non-Cargo packages released from this repo: name -> (versioned_files, changelog).
EXTRA_PACKAGES: dict[str, tuple[list[str], str]] = {
    "roder-sdk-typescript": (
        ["sdk/typescript/package.json", "sdk/typescript/package-lock.json"],
        "sdk/typescript/CHANGELOG.md",
    ),
    "roder-sdk-python": (
        ["sdk/python/pyproject.toml"],
        "sdk/python/CHANGELOG.md",
    ),
    "roder-edit-tools": (
        ["packages/edit-tools/package.json"],
        "packages/edit-tools/CHANGELOG.md",
    ),
}

HEADER = """\
# GENERATED FILE - regenerate with: python3 scripts/generate-knope-config.py
#
# Per-package versioning is driven by changesets in .changeset/ (see
# docs/releases.md). Adding a crate under crates/ requires regenerating
# this file; CI fails if it is stale.
"""

FOOTER = """\
# Versions are driven exclusively by changesets, never by commit messages.
[changes]
ignore_conventional_commits = true

# Interactive helper: `knope document-change` writes a .changeset/*.md file.
[[workflows]]
name = "document-change"

[[workflows.steps]]
type = "CreateChangeFile"

# Run on pushes to master: opens/updates the release preview PR.
[[workflows]]
name = "prepare-release"

[[workflows.steps]]
type = "Command"
command = "git switch -c knope/release"

[[workflows.steps]]
type = "PrepareRelease"

[[workflows.steps]]
type = "Command"
command = "git commit -m \\"chore: prepare release\\""

[[workflows.steps]]
type = "Command"
command = "git push --force --set-upstream origin knope/release"

[[workflows.steps]]
type = "CreatePullRequest"
base = "master"

[workflows.steps.title]
template = "chore: prepare release"

[workflows.steps.body]
template = "Merging this PR releases every package whose version changed here. Created by knope from the pending changesets."

# Run when the release preview PR merges: tags + GitHub release per package.
[[workflows]]
name = "release"

[[workflows.steps]]
type = "Release"

[github]
owner = "PandelisZ"
repo = "gode"
"""


def cargo_packages() -> list[tuple[str, Path]]:
    packages = []
    for manifest in sorted(CRATES_DIR.glob("*/Cargo.toml")):
        with manifest.open("rb") as f:
            data = tomllib.load(f)
        name = data["package"]["name"]
        packages.append((name, manifest.relative_to(REPO_ROOT)))
    return packages


def render() -> str:
    lines = [HEADER]
    for name, manifest in cargo_packages():
        crate_dir = manifest.parent.as_posix()
        lines.append(f'[packages."{name}"]')
        lines.append("versioned_files = [")
        lines.append(f'    "{manifest.as_posix()}",')
        lines.append('    "Cargo.lock",')
        lines.append("]")
        lines.append(f'changelog = "{crate_dir}/CHANGELOG.md"')
        lines.append("")
    for name, (files, changelog) in EXTRA_PACKAGES.items():
        lines.append(f'[packages."{name}"]')
        lines.append("versioned_files = [")
        for file in files:
            lines.append(f'    "{file}",')
        lines.append("]")
        lines.append(f'changelog = "{changelog}"')
        lines.append("")
    lines.append(FOOTER)
    return "\n".join(lines)


def main() -> int:
    content = render()
    if "--check" in sys.argv[1:]:
        current = KNOPE_TOML.read_text() if KNOPE_TOML.exists() else ""
        if current != content:
            print(
                "knope.toml is stale. Regenerate it with:\n"
                "    python3 scripts/generate-knope-config.py",
                file=sys.stderr,
            )
            return 1
        print("knope.toml is up to date.")
        return 0
    KNOPE_TOML.write_text(content)
    print(f"Wrote {KNOPE_TOML.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
