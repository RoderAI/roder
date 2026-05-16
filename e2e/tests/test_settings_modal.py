"""Settings modal interactions (ctrl+p toggle)."""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask gode to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_ctrl_p_opens_settings(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Settings", timeout=3)
    # Status line reflects the active modal.
    await tui.wait_for_predicate(
        lambda s: "settings" in s.row(s.rows - 1),
        timeout=2,
        description="status shows settings",
    )


async def test_settings_lists_known_options(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Settings", timeout=3)
    # These rows are stable across recent gode versions; if upstream
    # renames them, update here.
    for expected in ("Models", "Fast Mode", "Permission Mode"):
        assert expected in tui.screen.text, f"missing setting row: {expected}"


async def test_escape_closes_settings(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Settings", timeout=3)
    await tui.press("escape")
    await tui.wait_for_predicate(
        lambda s: "settings" not in s.row(s.rows - 1),
        timeout=3,
        description="status no longer shows settings",
    )
