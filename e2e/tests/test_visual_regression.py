"""Cell-grid snapshot regression for stable gode layouts.

These tests are intentionally narrow — they capture only the startup
screen and the settings modal at fixed dimensions. They will need a
``--snapshot-update`` run whenever the UI changes intentionally.
"""

from __future__ import annotations

import pytest
from syrupy.assertion import SnapshotAssertion

from tuiwright import TuiSession
from tuiwright._snapshot import ScreenSnapshotExtension

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str], *, cols: int, rows: int) -> None:
    await tui.start([gode_bin], env=gode_env, cols=cols, rows=rows)
    await tui.wait_for_text("Ask gode to work on this repo", timeout=_STARTUP_TIMEOUT)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)


async def test_startup_snapshot(
    tui: TuiSession,
    gode_bin: str,
    gode_env: dict[str, str],
    snapshot: SnapshotAssertion,
) -> None:
    await _ready(tui, gode_bin, gode_env, cols=120, rows=30)
    assert tui.screen == snapshot(extension_class=ScreenSnapshotExtension)


async def test_settings_modal_snapshot(
    tui: TuiSession,
    gode_bin: str,
    gode_env: dict[str, str],
    snapshot: SnapshotAssertion,
) -> None:
    await _ready(tui, gode_bin, gode_env, cols=120, rows=30)
    await tui.press("ctrl+p")
    await tui.wait_for_text("Settings", timeout=3)
    await tui.wait_for_stable(quiet_ms=200, timeout=3)
    assert tui.screen == snapshot(extension_class=ScreenSnapshotExtension)
