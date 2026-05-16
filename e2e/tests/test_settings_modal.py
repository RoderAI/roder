"""Provider/model menu interactions (ctrl+p toggle).

Roder's ctrl+p opens a "Menu" with Providers / Models options; this
file used to test an old "Settings" modal that has since been
replaced. The file name is kept for git history.
"""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_ctrl_p_opens_menu(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Menu", timeout=3)
    # The menu header advertises the controls.
    assert "Enter select" in tui.screen.text
    assert "Esc close" in tui.screen.text


async def test_menu_lists_known_options(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Menu", timeout=3)
    for expected in ("Providers", "Models"):
        assert expected in tui.screen.text, f"missing menu row: {expected}"


async def test_escape_closes_menu(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Menu", timeout=3)
    await tui.press("escape")
    await tui.wait_for_predicate(
        lambda s: "Menu (Enter select" not in s.text,
        timeout=3,
        description="menu closes",
    )


async def test_arrow_keys_navigate_menu(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """The triangle marker (›) sits next to the currently selected row."""
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Menu", timeout=3)
    # The first item (Providers) starts selected. Move down once.
    await tui.press("down")
    await tui.wait_for_stable(quiet_ms=120, timeout=2)
    # Now the marker should be on Models.
    # Find the row containing Models and confirm the marker char (›) is there.
    models_row = tui.screen.row_containing("Models")
    assert models_row is not None
    row_text = tui.screen.row(models_row)
    assert "›" in row_text, f"selection marker not on Models row: {row_text!r}"
    await tui.press("escape")
