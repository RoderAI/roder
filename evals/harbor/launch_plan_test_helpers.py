"""Shared fixtures for Harbor launch-plan validator tests."""

from __future__ import annotations

import json
from hashlib import sha256
from pathlib import Path

from pre_eval_config_summary import DEFAULT_CONFIGS, deadline_policy_summary
from pre_eval_harness_summary import DEFAULT_HARNESS_FILES
from tbench_diagnostic_test_data import passing_tbench_diagnostics_summary


def ready_plan() -> dict:
    return {
        "launchStatus": "ready",
        "blockedReasons": [],
        "dryRun": False,
        "wouldRunHarbor": True,
        "requireImagePreflight": True,
        "harborConfig": "evals/harbor/tbench-full-gpt55-medium.json",
        "imagePreflight": {
            "status": "passed",
            "config": "evals/harbor/tbench-full-gpt55-medium.json",
            "manifest": "evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json",
            "tasks": 89,
            "uniqueImages": 89,
            "present": 89,
            "missing": 0,
            "unresolved": 0,
            "pullFailed": 0,
            "selectionErrors": [],
            "blockedTasks": [],
        },
        "jobDir": "evals/harbor/jobs/roder-tbench-full-gpt55-medium",
        "analysisJson": "evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json",
        "analysisMarkdown": "evals/reports/harbor/roder-tbench-full-gpt55-medium.md",
        "preEvalSummary": "/tmp/pre-eval-summary.json",
        "harborConfigSha256": "a" * 64,
        "preEvalHarborConfigSha256": "a" * 64,
        "preEvalSummarySha256": "b" * 64,
        "deadlinePolicy": deadline_policy_summary(),
        "prebuiltBinary": {
            "path": "/tmp/roder-linux-amd64",
            "sha256": "c" * 64,
        },
        "authFile": {
            "path": "/tmp/codex.json",
            "validJson": True,
        },
        "harborHarness": {
            "status": "passed",
            "combinedSha256": "d" * 64,
            "entries": [
                {
                    "path": "evals/harbor/roder_harbor_agent.py",
                    "sha256": "e" * 64,
                }
            ],
        },
        "harborHarnessTests": {"status": "passed"},
        "preEvalSummaryStatus": {"status": "ok", "blockedChecks": []},
        "pullPreflight": False,
        "offlinePreflight": False,
    }


def executable_file(path: Path, contents: bytes) -> Path:
    path.write_bytes(contents)
    path.chmod(path.stat().st_mode | 0o100)
    return path


def linux_x86_64_elf_bytes() -> bytes:
    header = bytearray(64)
    header[0:4] = b"\x7fELF"
    header[4] = 2
    header[5] = 1
    header[7] = 3
    header[18:20] = (0x3E).to_bytes(2, "little")
    return bytes(header)


def auth_json() -> str:
    return (
        '{"access":"token","refresh":"refresh","account_id":"acct",'
        '"type":"bearer","expires":9999999999}\n'
    )


def combined_file_digest(entries: list[dict]) -> str:
    digest = sha256()
    for entry in sorted(entries, key=lambda item: str(item.get("path") or "")):
        digest.update(str(entry.get("path") or "").encode())
        digest.update(b"\0")
        digest.update(str(entry.get("sha256") or "").encode())
        digest.update(b"\0")
    return digest.hexdigest()


def clean_summary(
    *,
    prebuilt: Path,
    auth: Path,
    config: Path,
    image_preflight: bool = True,
) -> dict:
    config_sha = sha256(config.read_bytes()).hexdigest()
    harness_entries = harness_entries_from_defaults()
    summary = {
        "status": "ok",
        "blockedChecks": [],
        "options": {
            "runTests": True,
            "requirePrebuilt": True,
            "requireAuth": True,
            "preflightImages": image_preflight,
            "offlineImages": False,
            "pullImages": False,
            "imageConfig": str(config) if image_preflight else None,
        },
        "prebuiltBinary": {
            "required": True,
            "exists": True,
            "executable": True,
            "linuxX8664Elf": True,
            "path": str(prebuilt),
            "sha256": sha256(prebuilt.read_bytes()).hexdigest(),
        },
        "authFile": {
            "required": True,
            "exists": True,
            "validJson": True,
            "path": str(auth),
        },
        "checks": {
            "preEvalOptions": {"status": "passed", "issues": []},
            "harborReadiness": {"status": "passed"},
            "harborConfigs": {
                "status": "passed",
                "issues": [],
                "deadlinePolicy": deadline_policy_summary(),
                "entries": config_entries(config),
            },
            "harborHarness": {
                "status": "passed",
                "files": len(harness_entries),
                "issues": [],
                "entries": harness_entries,
                "combinedSha256": combined_file_digest(harness_entries),
            },
            "harborHarnessTests": {"status": "passed"},
            "roderEvalsLib": {"status": "passed"},
            "tbenchDiagnostics": passing_tbench_diagnostics_summary(),
        },
    }
    if image_preflight:
        summary["checks"]["imagePreflight"] = {
            "status": "passed",
            "offline": False,
            "config": str(config),
            "manifest": str(root_image_manifest(config)),
            "tasks": 89,
            "uniqueImages": 89,
            "present": 89,
            "missing": 0,
            "unresolved": 0,
            "pullFailed": 0,
            "selectionErrors": [],
            "blockedTasks": [],
        }
    return summary


def root_image_manifest(config: Path) -> Path:
    return config.parent / "image-preflight.json"


def harness_entries_from_defaults() -> list[dict]:
    return [
        {
            "path": str(path),
            "sha256": sha256(path.read_bytes()).hexdigest(),
        }
        for path in DEFAULT_HARNESS_FILES
    ]


def config_entries(extra_config: Path) -> list[dict]:
    by_path = {
        str(path): {
            "path": str(path),
            "sha256": sha256(path.read_bytes()).hexdigest(),
        }
        for path in DEFAULT_CONFIGS
    }
    by_path[str(extra_config)] = {
        "path": str(extra_config),
        "sha256": sha256(extra_config.read_bytes()).hexdigest(),
    }
    return list(by_path.values())


def write_clean_summary_fixture(
    root: Path,
    *,
    generated_at: str | None = None,
    image_preflight: bool = True,
) -> Path:
    prebuilt = executable_file(root / "roder-linux-amd64", linux_x86_64_elf_bytes())
    auth = root / "codex.json"
    auth.write_text(auth_json())
    config = root / "tbench.json"
    config.write_text('{"job_name":"test"}\n')
    summary_data = clean_summary(
        prebuilt=prebuilt,
        auth=auth,
        config=config,
        image_preflight=image_preflight,
    )
    if generated_at is not None:
        summary_data["generatedAt"] = generated_at
    summary = root / "pre-eval-summary.json"
    summary.write_text(json.dumps(summary_data) + "\n")
    return summary
