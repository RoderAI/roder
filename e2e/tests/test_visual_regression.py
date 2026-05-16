"""Cell-grid snapshot regression for stable Roder layouts.

These tests are intentionally narrow — they capture only the startup
screen and the settings modal at fixed dimensions. They will need a
``--snapshot-update`` run whenever the UI changes intentionally.
"""

from __future__ import annotations

import re
from dataclasses import replace

import pytest
from syrupy.assertion import SnapshotAssertion

from tuiwright.screen import Screen
from tuiwright import TuiSession
from tuiwright._snapshot import ScreenSnapshotExtension

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12
_SESSION_ID_RE = re.compile(r"session [0-9a-f]{8}")


async def _ready(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str], *, cols: int, rows: int
) -> None:
    await tui.start([gode_bin], env=gode_env, cols=cols, rows=rows)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)


def stable_screen(screen: Screen) -> Screen:
    rows = [list(row) for row in screen.cells]
    for row_index in range(screen.rows):
        row = screen.row_padded(row_index)
        for match in _SESSION_ID_RE.finditer(row):
            replacement = "session XXXXXXXX"
            for offset, char in enumerate(replacement):
                col = match.start() + offset
                rows[row_index][col] = replace(rows[row_index][col], char=char)
    return replace(screen, cells=tuple(tuple(row) for row in rows))


async def test_startup_snapshot(
    tui: TuiSession,
    gode_bin: str,
    gode_env: dict[str, str],
    snapshot: SnapshotAssertion,
) -> None:
    await _ready(tui, gode_bin, gode_env, cols=120, rows=30)
    assert stable_screen(tui.screen) == snapshot(extension_class=ScreenSnapshotExtension)


async def test_menu_modal_snapshot(
    tui: TuiSession,
    gode_bin: str,
    gode_env: dict[str, str],
    snapshot: SnapshotAssertion,
) -> None:
    await _ready(tui, gode_bin, gode_env, cols=120, rows=30)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Menu", timeout=3)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)
    assert stable_screen(tui.screen) == snapshot(extension_class=ScreenSnapshotExtension)
