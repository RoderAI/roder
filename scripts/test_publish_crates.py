#!/usr/bin/env python3
"""Regression tests for the workspace crate publish order."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("publish-crates.py")
SPEC = importlib.util.spec_from_file_location("publish_crates", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
publish_crates = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(publish_crates)


def package(
    name: str,
    *,
    dependencies: list[dict[str, str]] | None = None,
    publish: list[str] | None = None,
) -> dict[str, object]:
    value: dict[str, object] = {
        "id": f"path+file:///{name}#0.1.0",
        "name": name,
        "dependencies": dependencies or [],
    }
    if publish is not None:
        value["publish"] = publish
    return value


class PublishOrderTests(unittest.TestCase):
    def test_duplicate_normal_and_dev_dependency_is_one_graph_edge(self) -> None:
        dependency = package("dependency")
        dependent = package(
            "dependent",
            dependencies=[
                {"name": "dependency", "kind": None},
                {"name": "dependency", "kind": "dev"},
            ],
        )
        metadata = {
            "packages": [dependent, dependency],
            "workspace_members": [dependent["id"], dependency["id"]],
        }

        self.assertEqual(
            publish_crates.publish_order_from_metadata(metadata),
            ["dependency", "dependent"],
        )

    def test_cycle_fails_instead_of_silently_omitting_members(self) -> None:
        first = package("first", dependencies=[{"name": "second"}])
        second = package("second", dependencies=[{"name": "first"}])
        metadata = {
            "packages": [first, second],
            "workspace_members": [first["id"], second["id"]],
        }

        with self.assertRaisesRegex(RuntimeError, "first, second"):
            publish_crates.publish_order_from_metadata(metadata)

    def test_current_workspace_includes_every_publishable_member(self) -> None:
        metadata = json.loads(
            subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1"],
                cwd=publish_crates.REPO_ROOT,
                text=True,
            )
        )
        expected = {
            item["name"]
            for item in metadata["packages"]
            if item["id"] in metadata["workspace_members"]
            and publish_crates.is_publishable_to_crates_io(item)
        }

        self.assertEqual(set(publish_crates.publish_order_from_metadata(metadata)), expected)


if __name__ == "__main__":
    unittest.main()
