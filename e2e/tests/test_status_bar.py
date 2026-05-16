"""Top + bottom status-bar content assertions.

The top status row shows the app name, current model, reasoning level,
and a session id. The bottom row carries the keybinding hints and the
current mode. Both are stable surfaces that should not silently shift.
"""

from __future__ import annotations

import re

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)
    await tui.wait_for_stable(quiet_ms=120, timeout=3)


class TestTopStatus:
    async def test_app_name(self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
        await _ready(tui, gode_bin, gode_env)
        assert "roder" in tui.screen.row(0)

    async def test_provider_and_model(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        top = tui.screen.row(0)
        # Provider/model pair always shows as <provider>/<model>.
        assert re.search(r"\w+/\S+", top), f"no provider/model in: {top!r}"

    async def test_session_id_present(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        top = tui.screen.row(0)
        # 8 hex chars after "session ".
        assert re.search(r"session [0-9a-f]{8}", top), f"no session id in: {top!r}"

    async def test_idle_indicator(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        """The right-hand 'idle' marker shows when no turn is active."""
        await _ready(tui, gode_bin, gode_env)
        assert "idle" in tui.screen.row(0)


class TestBottomStatus:
    async def test_shows_ready(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        assert "ready" in tui.screen.row(tui.screen.rows - 1)

    async def test_shows_mode_default(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        assert "mode:default" in tui.screen.row(tui.screen.rows - 1)

    async def test_advertises_core_keybindings(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        """The footer is the discoverability surface — these hints must be present."""
        await _ready(tui, gode_bin, gode_env)
        footer = tui.screen.row(tui.screen.rows - 1)
        for hint in ("enter send", "shift+enter newline", "tab timeline", "ctrl+p"):
            assert hint in footer, f"missing hint {hint!r} in footer: {footer!r}"


class TestComposerFrame:
    async def test_default_frame_unlabeled(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        """In default mode the composer's top border has no title text."""
        await _ready(tui, gode_bin, gode_env)
        # Find the row just above the composer placeholder.
        placeholder = tui.screen.row_containing("Ask Roder to work on this repo")
        assert placeholder is not None
        border = tui.screen.row(placeholder - 1)
        # Plain dashes only — no mode label.
        assert "accept_edits" not in border
        assert "shell" not in border
