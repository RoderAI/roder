"""Policy mode + shell mode tests.

shift+tab cycles between policy modes (default → accept_edits → ...).
Pressing ``!`` in default mode toggles a transient shell-prompt mode
that wraps the composer with a ``shell`` label and prefixes the buffer.
"""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)


class TestPolicyMode:
    async def test_shift_tab_switches_to_accept_edits(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        await tui.press("shift+tab")
        # The transcript prints a banner and the composer border picks
        # up the mode name.
        await tui.wait_for_text("policy mode set to accept_edits", timeout=3)
        await tui.wait_for_text("accept_edits", timeout=2)

    async def test_status_bar_reflects_mode(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        await tui.press("shift+tab")
        await tui.wait_for_text("policy mode set to accept_edits")
        await tui.wait_for_stable(quiet_ms=120)
        assert "mode:accept_edits" in tui.screen.row(tui.screen.rows - 1)

    async def test_mode_cycles_back_to_default(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        """Several shift+tab presses should eventually land back on default."""
        await _ready(tui, gode_bin, gode_env)
        for _ in range(6):
            await tui.press("shift+tab")
            await tui.wait_for_stable(quiet_ms=120, timeout=2)
            if "mode:default" in tui.screen.row(tui.screen.rows - 1):
                return
        raise AssertionError("never returned to mode:default after 6 shift+tab presses")


class TestShellMode:
    async def test_bang_opens_shell_mode(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        await tui.type("!")
        # Composer border now reads "shell".
        await tui.wait_for_predicate(
            lambda s: any(
                "shell" in s.row(i) and ("╭" in s.row(i) or "│" in s.row(i))
                for i in range(s.rows)
            ),
            timeout=3,
            description="shell-labeled composer border",
        )
        # Footer advertises the new mode.
        await tui.wait_for_predicate(
            lambda s: "shell mode" in s.row(s.rows - 1),
            timeout=2,
            description="footer shows 'shell mode'",
        )

    async def test_bang_followed_by_command_visible(
        self, tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
    ) -> None:
        await _ready(tui, gode_bin, gode_env)
        await tui.type("!ls -la")
        await tui.wait_for_text("ls -la", timeout=3)
