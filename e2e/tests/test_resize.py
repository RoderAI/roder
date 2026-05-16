"""Resize behaviour: gode must reflow without crashing."""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask gode to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_shrink_to_80x24(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.resize(80, 24)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)
    assert tui.alive
    assert tui.screen.cols == 80
    assert tui.screen.rows == 24
    # Composer label survives the smaller width (may be truncated).
    assert "Ask" in tui.screen.text


async def test_grow_back_to_140x44(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.resize(80, 24)
    await tui.wait_for_stable(quiet_ms=200)
    await tui.resize(140, 44)
    await tui.wait_for_stable(quiet_ms=200)
    assert tui.alive
    assert tui.screen.cols == 140


async def test_tiny_terminal_does_not_crash(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """40x10 is below most TUI minimums; the app should degrade, not panic."""
    await _ready(tui, gode_bin, gode_env)
    await tui.resize(40, 10)
    await tui.wait_for_stable(quiet_ms=300, timeout=4)
    assert tui.alive, "gode crashed on extreme resize"


async def test_rapid_resize_burst(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """Drag-resize stresses the app with a burst of SIGWINCH."""
    await _ready(tui, gode_bin, gode_env)
    for cols in (130, 120, 110, 100, 110, 120, 130, 140):
        await tui.resize(cols, 44)
    await tui.wait_for_stable(quiet_ms=300, timeout=4)
    assert tui.alive
    assert tui.screen.cols == 140
