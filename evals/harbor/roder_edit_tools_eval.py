#!/usr/bin/env python3
"""Offline edit-tools fixture runner.

This intentionally avoids provider keys and npm publishing. It exercises the
same old/new-string and Codex-style patch semantics tracked by roadmap 80.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from tempfile import TemporaryDirectory


def apply_edit(text: str, old: str, new: str) -> tuple[str | None, str | None]:
    count = text.count(old)
    if count == 0:
        return None, "old_string_not_found"
    if count > 1:
        return None, "old_string_ambiguous"
    return text.replace(old, new, 1), None


def apply_patch(root: Path, patch: str) -> str | None:
    lines = patch.replace("\r\n", "\n").split("\n")
    if not lines or lines[0].strip() != "*** Begin Patch":
        return "apply_patch_failed"
    i = 1
    while i < len(lines):
        line = lines[i]
        if line == "*** End Patch":
            return None
        if not line.startswith("*** Update File: "):
            return "apply_patch_failed"
        rel = line.removeprefix("*** Update File: ").strip()
        path = root / rel
        text = path.read_text()
        i += 1
        while i < len(lines) and not lines[i].startswith("*** "):
            if not lines[i].startswith("@@"):
                return "apply_patch_failed"
            old_lines: list[str] = []
            new_lines: list[str] = []
            i += 1
            while i < len(lines) and not lines[i].startswith("@@") and not lines[i].startswith("*** "):
                hunk_line = lines[i]
                body = hunk_line[1:]
                if hunk_line.startswith(" "):
                    old_lines.append(body)
                    new_lines.append(body)
                elif hunk_line.startswith("-"):
                    old_lines.append(body)
                elif hunk_line.startswith("+"):
                    new_lines.append(body)
                else:
                    return "apply_patch_failed"
                i += 1
            old_text = "\n".join(old_lines)
            new_text = "\n".join(new_lines)
            if old_text not in text:
                return "apply_patch_failed"
            text = text.replace(old_text, new_text, 1)
        path.write_text(text)
    return "apply_patch_failed"


def run_fixture(fixture: dict[str, object]) -> dict[str, object]:
    with TemporaryDirectory(prefix="roder-edit-eval-") as tmp:
        root = Path(tmp)
        file_path = root / "fixture.txt"
        file_path.write_text(str(fixture["initial"]))
        operation = fixture["operation"]
        assert isinstance(operation, dict)
        error: str | None = None
        if operation["kind"] == "edit":
            updated, error = apply_edit(file_path.read_text(), str(operation["old_string"]), str(operation["new_string"]))
            if updated is not None:
                file_path.write_text(updated)
        elif operation["kind"] == "multi_edit":
            text = file_path.read_text()
            for edit in operation["edits"]:  # type: ignore[index]
                updated, error = apply_edit(text, str(edit["old_string"]), str(edit["new_string"]))
                if error:
                    break
                assert updated is not None
                text = updated
            if not error:
                file_path.write_text(text)
        elif operation["kind"] == "patch":
            error = apply_patch(root, str(operation["patch"]))
        else:
            error = "unknown_operation"
        expected_error = fixture.get("expect_error")
        if expected_error:
            passed = error == expected_error
        else:
            passed = error is None and file_path.read_text() == fixture["expected"]
        return {"name": fixture["name"], "passed": passed, "error": error}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixtures", default="evals/fixtures/edit-tools")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    fixture_dir = Path(args.fixtures)
    results = []
    for path in sorted(fixture_dir.glob("*.json")):
        data = json.loads(path.read_text())
        results.extend(run_fixture(fixture) for fixture in data["fixtures"])
    print(json.dumps({"offline": True, "results": results}, indent=2))
    return 0 if all(result["passed"] for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
