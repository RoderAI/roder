"""Test helpers for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
GENERATE = ROOT / "evals/harbor/generate_tbench_campaign.py"
VALIDATE = ROOT / "evals/harbor/validate_tbench_campaign.py"


def generate_campaign(output_dir: Path) -> Path:
    result = subprocess.run(
        [
            "python3",
            str(GENERATE),
            "--route",
            "xhigh-validated",
            "--output-dir",
            str(output_dir),
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return output_dir / "validated-conversions-manifest.json"


def validate_campaign(manifest: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(VALIDATE), str(manifest)],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
