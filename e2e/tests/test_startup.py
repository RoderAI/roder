"""Startup and initial-render assertions for gode."""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio


# gode takes a couple of seconds to render the first frame (provider init,
# config load, app-server handshake). Tests that wait for it use this.
_STARTUP_TIMEOUT = 12


async def _wait_ready(tui: TuiSession) -> None:
    # Wait for the composer placeholder — the last thing painted on
    # startup — then settle so the status line at the bottom is drawn too.
    await tui.wait_for_text("Ask gode to work on this repo", timeout=_STARTUP_TIMEOUT)
    await tui.wait_for_stable(quiet_ms=150, timeout=3)


async def test_binary_launches(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await _wait_ready(tui)
    assert tui.alive


async def test_status_line_shows_ready(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await _wait_ready(tui)
    status = tui.screen.row(tui.screen.rows - 1)
    assert "ready" in status, f"expected 'ready' in status line, got: {status!r}"
    assert "ctx" in status, "status line should show context usage"


async def test_top_status_shows_app_name(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await _wait_ready(tui)
    top = tui.screen.row(0)
    assert "gode" in top


async def test_welcome_message_visible(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("No transcript yet", timeout=_STARTUP_TIMEOUT)


async def test_composer_placeholder_visible(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask gode to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_enables_mouse_and_bracketed_paste(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """gode should enable SGR mouse + bracketed paste during startup."""
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await _wait_ready(tui)
    # The emulator's mode-tracking should observe gode's CSI ? h sequences.
    # SGR 1006 (mouse) and 2004 (paste) are required for the TUI to work.
    await tui.wait_for_predicate(
        lambda _: (
            tui._emu.is_mouse_tracking()  # type: ignore[attr-defined]
            and tui._emu.is_bracketed_paste()  # type: ignore[attr-defined]
        ),
        timeout=3,
        description="mouse + paste modes enabled",
    )
