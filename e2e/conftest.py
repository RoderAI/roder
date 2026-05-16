"""Shared fixtures for gode end-to-end tests."""

from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BIN = REPO_ROOT / "bin" / "gode"


@pytest.fixture
def gode_bin() -> str:
    """Resolve the gode binary path.

    Resolution order:
        1. ``GODE_BIN`` env var
        2. ``/Users/pz/w/gode/bin/gode`` (the repo-local committed build)
        3. ``gode`` on PATH

    Skips the test if none are usable so this suite can run in environments
    where the binary hasn't been built.
    """
    candidates: list[str | None] = [
        os.environ.get("GODE_BIN"),
        str(DEFAULT_BIN) if DEFAULT_BIN.is_file() else None,
        shutil.which("gode"),
    ]
    for c in candidates:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    pytest.skip(
        "no gode binary found: set GODE_BIN, build with `cargo build -p roder-cli`, "
        "or put `gode` on PATH"
    )


@pytest.fixture
def gode_env() -> dict[str, str]:
    """Env vars given to every gode subprocess.

    A dummy ``OPENAI_API_KEY`` keeps the provider init from failing on
    machines without real credentials. ``GODE_TEST_MODE`` is a soft hint
    that some flows in gode check.
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
