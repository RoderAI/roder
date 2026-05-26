"""Shared fixtures for pre-eval summary validator tests."""

from __future__ import annotations

from tbench_diagnostic_test_data import passing_tbench_diagnostics_summary


def clean_summary() -> dict:
    return {
        "status": "ok",
        "blockedChecks": [],
        "options": {
            "runTests": True,
            "requirePrebuilt": True,
            "requireAuth": True,
            "preflightImages": True,
            "imageConfig": "evals/harbor/tbench-full-gpt55-medium.json",
            "analysisTarget": "evals/reports/harbor/full-analysis.json",
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
            "harborReadiness": {"status": "passed"},
            "harborConfigs": {"status": "passed", "issues": []},
            "harborHarness": {
                "status": "passed",
                "files": 1,
                "issues": [],
                "combinedSha256": "a" * 64,
                "entries": [
                    {
                        "path": "evals/harbor/roder_harbor_agent.py",
                        "sha256": "b" * 64,
                        "sizeBytes": 1,
                    }
                ],
            },
            "harborHarnessTests": {"status": "passed"},
            "roderEvalsLib": {"status": "passed"},
            "tbenchDiagnostics": passing_tbench_diagnostics_summary(),
            "imagePreflight": {
                "status": "passed",
                "config": "evals/harbor/tbench-full-gpt55-medium.json",
                "manifest": "/tmp/image-preflight-manifest.json",
                "tasks": 4,
                "uniqueImages": 4,
                "present": 4,
                "missing": 0,
                "unresolved": 0,
                "pullFailed": 0,
                "selectionErrors": [],
                "blockedTasks": [],
            },
            "harborAnalysisBaseline": {"status": "ok"},
        },
    }
