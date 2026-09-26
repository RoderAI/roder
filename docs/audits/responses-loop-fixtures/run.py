#!/usr/bin/env python3
"""Run offline contract probes against a copied Responses provider and real core.

The provider regressions run against the current source; runtime regressions run
against the current core implementation. Production sources and the repository
Cargo.lock are never edited. Requires Python 3.11+. Original probe source files
are retained as evidence of the baseline audit.
"""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def toml_value(value):
    if isinstance(value, dict):
        fields = (f"{key} = {toml_value(item)}" for key, item in value.items())
        return "{ " + ", ".join(fields) + " }"
    return json.dumps(value)


def main():
    fixtures = Path(__file__).resolve().parent
    repo = fixtures.parents[2]
    provider = repo / "crates/roder-ext-openai-responses"
    scratch = Path(tempfile.mkdtemp(prefix="roder-responses-audit-"))
    root_config = tomllib.loads((repo / "Cargo.toml").read_text())
    leaf_config = tomllib.loads((provider / "Cargo.toml").read_text())
    shared = root_config["workspace"]["dependencies"]
    dependencies = dict(leaf_config["dependencies"])
    dependencies["roder-core"] = {"workspace": True}
    lines = [
        "[workspace]",
        "[package]",
        'name = "roder-responses-loop-audit"',
        'version = "0.0.0"',
        f'edition = {toml_value(root_config["workspace"]["package"]["edition"])}',
        "[dependencies]",
    ]
    for name, leaf in dependencies.items():
        if isinstance(leaf, dict) and leaf.get("workspace"):
            entry = shared[name]
            dependency = {"version": entry} if isinstance(entry, str) else dict(entry)
            if "path" in dependency:
                dependency["path"] = str(repo / dependency["path"])
            for key, value in leaf.items():
                if key == "workspace":
                    continue
                if key == "features":
                    value = sorted(set(dependency.get(key, []) + value))
                dependency[key] = value
        else:
            dependency = {"version": leaf} if isinstance(leaf, str) else dict(leaf)
        lines.append(f"{name} = {toml_value(dependency)}")
    for name in ("dev", "test"):
        lines.append(f"[profile.{name}]")
        for key, value in root_config["profile"][name].items():
            lines.append(f"{key} = {toml_value(value)}")
    (scratch / "Cargo.toml").write_text("\n".join(lines) + "\n")
    shutil.copy(repo / "Cargo.lock", scratch / "Cargo.lock")
    shutil.copytree(provider / "src", scratch / "src")
    shutil.copytree(provider / "assets", scratch / "assets")
    (scratch / "tests").mkdir()
    shutil.copy(repo / "crates/roder-core/tests/responses_loop.rs", scratch / "tests/runtime_audit.rs")
    source = scratch / "src/provider.rs"
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    print(f"Scratch harness: {scratch}\nProvider source SHA256: {digest}", flush=True)
    command = ["mise", "exec", "--", "cargo", "test", "--offline", "--manifest-path", str(scratch / "Cargo.toml")]
    results = []
    for target in (["--lib", "audit_"], ["--test", "runtime_audit"]):
        results.append(subprocess.run(command + target + ["--", "--nocapture"], cwd=repo).returncode)
    return int(any(results))


if __name__ == "__main__":
    raise SystemExit(main())
