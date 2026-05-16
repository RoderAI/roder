"""Mouse interactions: scroll wheel, clicks.

Roder enables SGR mouse (1006) + any-event mouse (1003) on startup,
so click / scroll / hover all work as soon as the composer is ready.
"""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)
    # Mouse must be live before we click — wait for the DEC modes.
    await tui.wait_for_predicate(
        lambda _: tui._emu.is_mouse_tracking(),  # type: ignore[attr-defined]
        timeout=3,
        description="mouse tracking enabled",
    )


async def test_click_on_composer_does_not_crash(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    placeholder = tui.screen.row_containing("Ask Roder to work on this repo")
    assert placeholder is not None
    await tui.click(row=placeholder, col=10)
    await tui.wait_for_stable(quiet_ms=120, timeout=2)
    assert tui.alive


async def test_scroll_wheel_in_transcript(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """Scrolling in the transcript area shouldn't crash; the app may move
    a scroll indicator but should otherwise stay alive."""
    await _ready(tui, gode_bin, gode_env)
    # Scroll in the middle of the screen (transcript area).
    for direction in ("up", "down", "up", "up"):
        await tui.scroll(row=15, col=70, direction=direction)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)
    assert tui.alive
    # Composer placeholder should still be visible.
    assert "Ask Roder to work on this repo" in tui.screen.text


async def test_click_does_not_consume_typed_text(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """After clicking and then typing, the typed text should land in the
    composer — confirms focus is back on the input."""
    await _ready(tui, gode_bin, gode_env)
    placeholder = tui.screen.row_containing("Ask Roder to work on this repo")
    assert placeholder is not None
    await tui.click(row=placeholder, col=20)
    await tui.wait_for_stable(quiet_ms=120, timeout=2)
    await tui.type("typed after click")
    await tui.wait_for_text("typed after click", timeout=3)


async def test_rapid_scroll_burst(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """A burst of wheel events should not crash or corrupt the layout."""
    await _ready(tui, gode_bin, gode_env)
    for _ in range(10):
        await tui.scroll(row=10, col=70, direction="down")
    for _ in range(10):
        await tui.scroll(row=10, col=70, direction="up")
    await tui.wait_for_stable(quiet_ms=300, timeout=3)
    assert tui.alive
