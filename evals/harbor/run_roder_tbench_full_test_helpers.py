"""Shared fixtures for run-roder-tbench-full wrapper tests."""

from __future__ import annotations

import json
import stat
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path

from pre_eval_config_summary import DEFAULT_CONFIGS, deadline_policy_summary
from pre_eval_harness_summary import DEFAULT_HARNESS_FILES
from tbench_diagnostic_test_data import passing_tbench_diagnostics_summary


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "evals/harbor/run-roder-tbench-full.sh"
HARNESS_FILES = DEFAULT_HARNESS_FILES
CONFIG_FILES = DEFAULT_CONFIGS


def linux_x86_64_elf_header() -> bytes:
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4] = 2
    header[5] = 1
    header[7] = 3
    header[18:20] = (0x3E).to_bytes(2, "little")
    return bytes(header)


def harness_summary() -> dict:
    entries = []
    for relative in HARNESS_FILES:
        path = ROOT / relative
        entries.append(
            {
                "path": relative.as_posix(),
                "sha256": sha256(path.read_bytes()).hexdigest(),
                "sizeBytes": path.stat().st_size,
            }
        )
    digest = sha256()
    for entry in sorted(entries, key=lambda item: item["path"]):
        digest.update(entry["path"].encode())
        digest.update(b"\0")
        digest.update(entry["sha256"].encode())
        digest.update(b"\0")
    return {
        "status": "passed",
        "files": len(entries),
        "issues": [],
        "combinedSha256": digest.hexdigest(),
        "entries": entries,
    }


def config_entries() -> list[dict]:
    return [
        {
            "path": relative.as_posix(),
            "sha256": sha256((ROOT / relative).read_bytes()).hexdigest(),
        }
        for relative in CONFIG_FILES
    ]


def clean_summary(
    *,
    preflight_images: bool = False,
    prebuilt_binary: Path | None = None,
    auth_file: Path | None = None,
) -> dict:
    if prebuilt_binary is not None:
        prebuilt_binary.write_bytes(linux_x86_64_elf_header())
        prebuilt_binary.chmod(prebuilt_binary.stat().st_mode | stat.S_IXUSR)
        if auth_file is None:
            auth_file = prebuilt_binary.parent / "codex.json"
    if auth_file is not None:
        auth_file.write_text(
            (
                '{"access":"redacted","refresh":"redacted",'
                '"account_id":"acct","type":"bearer","expires":1}\n'
            )
        )
    summary = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "status": "ok",
        "blockedChecks": [],
        "options": {
            "runTests": True,
            "requirePrebuilt": True,
            "requireAuth": True,
            "preflightImages": preflight_images,
            "offlineImages": False,
            "pullImages": False,
            "imageConfig": (
                "evals/harbor/tbench-full-gpt55-medium.json"
                if preflight_images
                else None
            ),
            "analysisTarget": None,
        },
        "prebuiltBinary": {
            "required": True,
            "exists": True,
            "executable": True,
            "linuxX8664Elf": True,
        },
        "authFile": {
            "required": True,
            "exists": True,
            "validJson": True,
        },
        "checks": {
            "preEvalOptions": {"status": "passed", "issues": []},
            "harborReadiness": {"status": "passed"},
            "harborConfigs": {
                "status": "passed",
                "issues": [],
                "deadlinePolicy": deadline_policy_summary(),
                "entries": config_entries(),
            },
            "harborHarness": harness_summary(),
            "harborHarnessTests": {"status": "passed"},
            "roderEvalsLib": {"status": "passed"},
            "tbenchDiagnostics": passing_tbench_diagnostics_summary(),
        },
    }
    if prebuilt_binary is not None:
        stat_result = prebuilt_binary.stat()
        summary["prebuiltBinary"].update(
            {
                "path": str(prebuilt_binary),
                "sizeBytes": stat_result.st_size,
                "sha256": sha256(prebuilt_binary.read_bytes()).hexdigest(),
            }
        )
    if auth_file is not None:
        stat_result = auth_file.stat()
        summary["authFile"].update(
            {
                "path": str(auth_file),
                "sizeBytes": stat_result.st_size,
                "validJson": True,
                "jsonFields": ["access", "account_id", "expires", "refresh", "type"],
            }
        )
    if preflight_images:
        add_clean_image_preflight(summary, prebuilt_binary)
    return summary


def add_clean_image_preflight(
    summary: dict,
    prebuilt_binary: Path | None,
) -> None:
    manifest = (
        prebuilt_binary.parent / "image-preflight-manifest.json"
        if prebuilt_binary is not None
        else Path("/tmp/image-preflight-manifest.json")
    )
    if prebuilt_binary is not None:
        manifest.write_text(
            json.dumps(
                {
                    "clean": True,
                    "config": "evals/harbor/tbench-full-gpt55-medium.json",
                    "offline": False,
                    "pull": False,
                    "summary": {
                        "tasks": 89,
                        "unique_images": 89,
                        "present": 89,
                        "missing": 0,
                        "unresolved": 0,
                        "pull_failed": 0,
                    },
                    "tasks": [
                        {
                            "task_name": f"tbench-task-{index}",
                            "status": "present",
                            "image": f"example/{index}:latest",
                        }
                        for index in range(89)
                    ],
                    "images": [
                        {
                            "image": f"example/{index}:latest",
                            "tasks": [f"tbench-task-{index}"],
                        }
                        for index in range(89)
                    ],
                }
            )
            + "\n"
        )
    summary["checks"]["imagePreflight"] = {
        "status": "passed",
        "offline": False,
        "config": "evals/harbor/tbench-full-gpt55-medium.json",
        "manifest": str(manifest),
        "tasks": 89,
        "uniqueImages": 89,
        "present": 89,
        "missing": 0,
        "unresolved": 0,
        "pullFailed": 0,
        "selectionErrors": [],
        "blockedTasks": [],
    }
