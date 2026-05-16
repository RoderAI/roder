"""Shared fixtures for Roder end-to-end tests."""

from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BIN = REPO_ROOT / "bin" / "roder"


@pytest.fixture
def gode_bin() -> str:
    """Resolve the Roder binary path.

    Resolution order:
        1. ``RODER_BIN`` env var
        2. ``GODE_BIN`` env var for older local test invocations
        3. ``/Users/pz/w/gode/bin/roder`` (the repo-local build)
        4. ``roder`` on PATH

    Skips the test if none are usable so this suite can run in environments
    where the binary hasn't been built.
    """
    candidates: list[str | None] = [
        os.environ.get("RODER_BIN"),
        os.environ.get("GODE_BIN"),
        str(DEFAULT_BIN) if DEFAULT_BIN.is_file() else None,
        shutil.which("roder"),
    ]
    for c in candidates:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    pytest.skip(
        "no Roder binary found: set RODER_BIN, build with `make build`, "
        "or put `roder` on PATH"
    )


@pytest.fixture
def gode_env() -> dict[str, str]:
    """Env vars given to every Roder subprocess.

    A dummy ``OPENAI_API_KEY`` keeps the provider init from failing on
    machines without real credentials. ``GODE_TEST_MODE`` is a soft hint
    that some flows in Roder check.
    """
    return {
        "OPENAI_API_KEY": os.environ.get("OPENAI_API_KEY", "sk-test"),
        "ANTHROPIC_API_KEY": os.environ.get("ANTHROPIC_API_KEY", "sk-ant-test"),
        "GEMINI_API_KEY": os.environ.get("GEMINI_API_KEY", "test-gemini"),
        "RUST_BACKTRACE": "1",
        "GODE_TEST_MODE": "1",
        # Force a stable HOME so the user-config lookup is deterministic.
        "HOME": os.environ.get("HOME", "/tmp"),
    }
