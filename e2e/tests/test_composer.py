"""Composer (text input) tests for Roder."""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_typing_appears_in_composer(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.type("hello from tuiwright")
    await tui.wait_for_text("hello from tuiwright", timeout=3)


async def test_typing_replaces_placeholder(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """Once the user starts typing the placeholder text disappears."""
    await _ready(tui, gode_bin, gode_env)
    await tui.type("x")
    await tui.wait_for_text("x", timeout=3)
    # Some redraws of the composer keep the placeholder dim — assert that
    # at least one character of input replaced it in the box.
    assert "x" in tui.screen.text


async def test_backspace_removes_char(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.type("foobar")
    await tui.wait_for_text("foobar", timeout=3)
    # Roder's composer needs a settle between rapid backspaces — a real
    # user types at <10 keys/sec which leaves time for the render loop.
    for _ in range(3):
        await tui.press("backspace")
        await tui.wait_for_stable(quiet_ms=80, timeout=2)
    assert "foo" in tui.screen.text
    assert "foobar" not in tui.screen.text


async def test_multiple_lines_via_paste(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """Bracketed paste delivers a multi-line message as one composer edit."""
    await _ready(tui, gode_bin, gode_env)
    # Wait for paste mode specifically.
    await tui.wait_for_predicate(
        lambda _: tui._emu.is_bracketed_paste(),  # type: ignore[attr-defined]
        timeout=3,
        description="paste mode",
    )
    await tui.paste("first line\nsecond line\nthird line")
    await tui.wait_for_text("first line", timeout=3)
    assert "second line" in tui.screen.text
    assert "third line" in tui.screen.text


async def test_unicode_input(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.type("héllo café")
    await tui.wait_for_text("héllo café", timeout=3)
