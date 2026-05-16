"""Exit confirmation dialog (Esc from composer)."""

from __future__ import annotations

import pytest

from tuiwright import TuiSession

pytestmark = pytest.mark.asyncio

_STARTUP_TIMEOUT = 12


async def _ready(tui: TuiSession, gode_bin: str, gode_env: dict[str, str]) -> None:
    await tui.start([gode_bin], env=gode_env, cols=140, rows=44)
    await tui.wait_for_text("Ask Roder to work on this repo", timeout=_STARTUP_TIMEOUT)


async def test_esc_opens_exit_dialog(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    await tui.wait_for_text("Exit Roder", timeout=3)
    assert "Close the TUI?" in tui.screen.text


async def test_dialog_offers_yes_and_no(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    # Wait for the dialog's Yes/No row specifically — the dialog header
    # appears a moment before the body rows finish drawing.
    await tui.wait_for_text("Yes", timeout=3)
    for option in ("Yes", "No", "Cancel"):
        assert option in tui.screen.text, f"missing exit dialog option {option!r}"


async def test_dialog_advertises_keys(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    # The hint line "Left / Right select   Enter choose   Y/N" is the
    # last row of the dialog — wait for it directly.
    await tui.wait_for_text("Enter choose", timeout=3)
    assert "Left" in tui.screen.text and "Right" in tui.screen.text
    assert "Y/N" in tui.screen.text


async def test_n_cancels_exit(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    await tui.wait_for_text("Exit Roder", timeout=3)
    await tui.press("n")
    await tui.wait_for_predicate(
        lambda s: "Exit Roder" not in s.text,
        timeout=3,
        description="exit dialog dismissed",
    )
    assert tui.alive


async def test_esc_dismisses_dialog(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    """A second Esc backs out of the exit dialog (treated as Cancel)."""
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    await tui.wait_for_text("Exit Roder", timeout=3)
    await tui.press("escape")
    await tui.wait_for_predicate(
        lambda s: "Exit Roder" not in s.text,
        timeout=3,
        description="dialog closes on second escape",
    )
    assert tui.alive


async def test_y_quits_app(
    tui: TuiSession, gode_bin: str, gode_env: dict[str, str]
) -> None:
    await _ready(tui, gode_bin, gode_env)
    await tui.press("escape")
    await tui.wait_for_text("Exit Roder", timeout=3)
    await tui.press("y")
    await tui.wait_for_predicate(
        lambda _: not tui.alive,
        timeout=5,
        description="roder exits",
    )
    assert not tui.alive
