#!/usr/bin/env python3
"""Changeset gate: require a .changeset/*.md entry for every package a PR touches.

Packages are read from knope.toml (the generated knope config). A PR that
modifies files inside a package directory must add or update a changeset that
names that package. Run from the repo root:

    python3 scripts/check-changesets.py --base origin/master

Bypass for a whole PR by applying the `skip-changeset` label (handled by the
GitHub workflow, not this script).
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CHANGESET_DIR = REPO_ROOT / ".changeset"
VALID_BUMPS = {"major", "minor", "patch"}


def load_package_dirs() -> dict[str, str]:
    """Map knope package name -> repo-relative package directory."""
    with (REPO_ROOT / "knope.toml").open("rb") as f:
        config = tomllib.load(f)
    dirs = {}
    for name, package in config.get("packages", {}).items():
        first = package["versioned_files"][0]
        path = first["path"] if isinstance(first, dict) else first
        dirs[name] = str(Path(path).parent.as_posix())
    return dirs


def git_lines(*args: str) -> list[str]:
    out = subprocess.run(
        ["git", *args], cwd=REPO_ROOT, check=True, capture_output=True, text=True
    ).stdout
    return [line for line in out.splitlines() if line.strip()]


def parse_changeset(path: Path) -> dict[str, str]:
    """Return {package: bump} from a change file's frontmatter."""
    lines = path.read_text().splitlines()
    if not lines or lines[0].strip() != "---":
        raise ValueError("missing frontmatter (file must start with ---)")
    bumps = {}
    for line in lines[1:]:
        if line.strip() == "---":
            return bumps
        if not line.strip():
            continue
        if ":" not in line:
            raise ValueError(f"invalid frontmatter line: {line!r}")
        package, _, bump = line.partition(":")
        bumps[package.strip().strip("\"'")] = bump.strip()
    raise ValueError("unterminated frontmatter (no closing ---)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/master", help="base ref to diff against")
    args = parser.parse_args()

    package_dirs = load_package_dirs()
    errors: list[str] = []

    # 1. Validate every pending change file (knope chokes on malformed ones).
    declared: dict[Path, dict[str, str]] = {}
    if CHANGESET_DIR.is_dir():
        for path in sorted(CHANGESET_DIR.iterdir()):
            if path.name.startswith("."):
                continue
            rel = path.relative_to(REPO_ROOT)
            if path.suffix != ".md":
                errors.append(f"{rel}: only .md change files belong in .changeset/")
                continue
            try:
                bumps = parse_changeset(path)
            except ValueError as e:
                errors.append(f"{rel}: {e}")
                continue
            if not bumps:
                errors.append(f"{rel}: frontmatter names no packages")
            for package, bump in bumps.items():
                if package not in package_dirs:
                    errors.append(f"{rel}: unknown package {package!r} (see knope.toml)")
                if bump not in VALID_BUMPS:
                    errors.append(
                        f"{rel}: invalid bump {bump!r} for {package!r}"
                        f" (expected major, minor, or patch)"
                    )
            declared[path] = bumps

    # 2. Find packages this PR touches and the changesets it adds/updates.
    merge_base = git_lines("merge-base", args.base, "HEAD")[0]
    changed_files = git_lines("diff", "--name-only", "--diff-filter=d", merge_base, "HEAD")

    touched: dict[str, list[str]] = {}
    covered: set[str] = set()
    for file in changed_files:
        if file.startswith(".changeset/"):
            path = REPO_ROOT / file
            covered.update(declared.get(path, {}))
            continue
        for package, package_dir in package_dirs.items():
            if file.startswith(package_dir + "/"):
                touched.setdefault(package, []).append(file)

    missing = sorted(set(touched) - covered)
    if missing:
        errors.append(
            "These packages changed without a changeset entry: " + ", ".join(missing)
        )
        errors.append(
            "Add a file like .changeset/short_description.md containing:\n"
            "    ---\n"
            + "".join(f"    {package}: patch\n" for package in missing)
            + "    ---\n"
            "\n"
            "    # One-line summary of the change\n"
            "(or run `knope document-change`; apply the `skip-changeset` PR label to bypass)"
        )

    if errors:
        print("Changeset gate failed:\n", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    if touched:
        print(f"Changeset gate passed: {', '.join(sorted(touched))} covered.")
    else:
        print("Changeset gate passed: no released packages touched.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
