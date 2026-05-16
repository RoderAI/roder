# Roder end-to-end TUI tests

Black-box tests that spawn the real `roder` binary under a pseudo-terminal
and drive it like a human would — typing, clicking, pasting, resizing.
Powered by [`tuiwright`](https://github.com/pandelisz/tuiwright).

## Setup

Build Roder (release recommended for speed):

```bash
make build
```

Install Python deps via `uv` (one-time, ~3 s):

```bash
cd e2e
uv sync --group dev
```

## Run

```bash
cd e2e
uv run pytest                                # all tests
uv run pytest tests/test_startup.py -v       # one file
uv run pytest -k "settings" -v               # one keyword
uv run pytest --snapshot-update              # refresh snapshots after a UI change
```

The test runner picks up `bin/roder` automatically. Override with:

```bash
RODER_BIN=/path/to/other/roder uv run pytest
```

## What's covered

| File | Scope |
|---|---|
| `test_startup.py` | Binary launches, status line, welcome message, DEC mode enablement |
| `test_composer.py` | Typing, backspace, multi-line paste, unicode input |
| `test_settings_modal.py` | Ctrl-P opens / Escape closes, expected setting rows |
| `test_resize.py` | Shrink to 80×24, grow back, tiny terminals, rapid SIGWINCH burst |
| `test_visual_regression.py` | Cell-grid snapshots of stable layouts |

## Tips

- Failing tests dump the full annotated screen to the report — read it
  first, then look at `pytest --tb=long` for the call site.
- The asciinema cast file for any failing test is retained under
  `pytest`'s `tmp_path`. Print the path with `pytest --tui-trace-dir=.`.
- For a flaky test, run it 20 times: `uv run pytest tests/test_x.py --count=20`
  (requires `pytest-repeat`).
