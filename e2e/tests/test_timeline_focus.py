"""Timeline focus: pressing tab moves focus from composer into the
transcript timeline. The bottom-bar key hints change accordingly, and
escape returns to the composer.
"""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_tab_focuses_timeline(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("tab")
    # The footer hint set switches to timeline-mode bindings.
    await tui.wait_for_text("j/k navigate", timeout=3)
    footer = tui.screen.row(tui.screen.rows - 1)
    for hint in ("j/k navigate", "pgup/pgdn scroll", "enter expand", "esc composer"):
        assert hint in footer, f"missing timeline hint {hint!r}"


async def test_esc_returns_to_composer_from_timeline(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("tab")
    await tui.wait_for_text("j/k navigate", timeout=3)
    await tui.press("escape")
    # Composer hints are back.
    await tui.wait_for_text("enter send", timeout=3)
    footer = tui.screen.row(tui.screen.rows - 1)
    assert "j/k navigate" not in footer


async def test_typing_re_focuses_composer(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """After tabbing to timeline, escape then type lands in the composer."""
    await _ready(tui, gode_bin, gode_env)
    await tui.press("tab")
    await tui.wait_for_text("j/k navigate", timeout=3)
    await tui.press("escape")
    await tui.wait_for_text("enter send", timeout=3)
    await tui.type("hello")
    await tui.wait_for_text("hello", timeout=3)
