# Roder Webwright Browser Agent

Roder Webwright is the first-party browser automation workflow that follows Microsoft's Webwright contract while keeping Roder as the host agent. The user-facing modes are `/webwright:run` for one-shot web tasks and `/webwright:craft` for reusable, parameterized CLI scripts.

Reference checked on 2026-05-26: `microsoft/Webwright` commit `29fc4b46c2827ac93168ac2f74c404e43d819562`.

## Contract Snapshot

- Upstream plugin id: `webwright`.
- Upstream plugin surface: Codex and Claude plugin manifests, a `skills/webwright/SKILL.md` skill, and command templates for `webwright:run` and `webwright:craft`.
- Runtime model: local Playwright scripts and a durable workspace, not persistent browser state.
- Required workspace files: `plan.md`, `final_script.py`, `final_runs/run_<id>/final_script.py`, `final_runs/run_<id>/screenshots/final_execution_*.png`, and `final_runs/run_<id>/final_script_log.txt`.
- One-shot mode solves the literal task values supplied by the user.
- Craft mode produces an import-safe Python CLI with concrete task values as defaults and a `--help` contract.
- Task2UI mode uses `task.json` and `report.json` for renderer-ready task reports.

## Roder Shape

Roder owns the host loop, selected model, tool policy, process tracking, and app-server/TUI visibility. The `roder-ext-webwright` extension owns the workspace contract, task executor, helper tools, artifact parsing, and offline verification helpers.

Normal local tests use fixture workspaces in `evals/fixtures/webwright` and do not require a browser, network, or external model API key. Live browser checks must opt in with `RODER_WEBWRIGHT_LIVE=1` and an explicit start URL.

## App-Server And CLI

The app-server exposes the Webwright workflow through `webwright/setup`, `webwright/prepare`, `webwright/submit`, `webwright/artifacts`, `webwright/latestRun`, `webwright/verify`, `webwright/report`, `webwright/rerun`, `webwright/export`, and `webwright/visualJudge`. These methods return structured setup, workspace, run, report, verification, visual-judge, and task-handle JSON so clients can display Webwright state without scraping terminal output.

The CLI mirrors the same surface:

```sh
roder webwright setup --browser firefox
roder webwright setup --browser chromium
roder webwright setup --browser webkit --dry-run
roder webwright run "Open the fixture page"
roder webwright run --browser chromium "Open the fixture page"
roder webwright craft "Download the report for account 123"
roder webwright inspect .roder/webwright/fixture-page
roder webwright verify .roder/webwright/fixture-page
roder webwright visual-judge .roder/webwright/fixture-page
roder webwright rerun .roder/webwright/fixture-page
roder webwright export .roder/webwright/fixture-page .roder/webwright-exports/fixture-page
```

In the TUI, `/webwright inspect <workspace>` renders critical-point status, latest-run screenshots, log tail, validation errors, and the final datum. `/webwright tail <workspace>` shows just the latest retained log tail.

## Helper Tools

The `webwright` tool provider exposes small contract helpers to the model:

- `webwright.prepare_workspace`: creates `plan.md`, `final_script.py`, and `webwright.json`.
- `webwright.allocate_run`: creates the next `final_runs/run_<id>/` directory and copies the current final script.
- `webwright.lint_script`: rejects full-page screenshots and scripts without an import-safe `__main__` guard.
- `webwright.run_script`: executes the copied final script through the current process-runner policy gate, using the managed Webwright runtime when no `python` override is passed.
- `webwright.list_artifacts`: returns the structured workspace summary.
- `webwright.read_log_tail`: reads a redacted tail of the latest `final_script_log.txt`.
- `webwright.verify_run`: fails the tool call when deterministic verification fails.
- `webwright.summarize_verification`: returns the same verification state without marking the tool call failed.

These helpers do not replace normal Roder file, edit, shell, media, and artifact tools. They remove repetitive Webwright ceremony while keeping all paths scoped to the current workspace.

## Artifact Layout

Every Webwright workspace uses this shape:

```text
.roder/webwright/<task-id>/
  webwright.json
  plan.md
  final_script.py
  task.json
  report.json
  visual_judge/
    run_001.json
  final_runs/
    run_001/
      final_script.py
      final_script_log.txt
      screenshots/
        final_execution_001_<label>.png
```

`task.json` and `report.json` are optional Task2UI files. Roder parses them when present and `webwright/report` returns both structured JSON and a redacted `renderedText` fallback so app-server and TUI clients can show reports without launching the upstream Flask viewer.

## Export

`webwright/export` and `roder webwright export` create a sanitized share directory for a task workspace. The exporter copies the Webwright manifest, plan, root script, Task2UI JSON, latest run scripts/logs, `self_reflect_result.json`, and `final_execution_*.png` screenshots. Text artifacts are redacted line-by-line for common secret patterns, and non-contract files such as cookies, browser state, raw headers, and unrelated workspace files are reported as excluded instead of copied.

## Visual Judge

`webwright/visualJudge` and `roder webwright visual-judge` are optional and disabled by default. They use the active Roder inference provider only when image input is supported and the method is explicitly enabled, then store the redacted prompt and provider response under `visual_judge/run_<n>.json` in the task workspace. If disabled or the active provider is text-only, Roder writes a skipped record instead of sending the screenshot.

## Local Browser Setup

The intended local runtime is Python 3.10+ with Playwright for Python. Roder can create and reuse a controlled user-level runtime:

```sh
roder webwright setup --browser firefox
```

Setup creates `~/.roder/python/webwright/venv`, installs the Playwright Python package, installs the selected browser (`firefox`, `chromium`, or `webkit`), and writes `~/.roder/python/webwright/setup.json`. If `RODER_CONFIG_DIR` or `RODER_DATA_DIR` is set, Roder uses that directory instead of `~/.roder`. Use `--dry-run` to inspect the exact commands without running them, and `--python /path/to/python3` to choose the base interpreter used for `python -m venv`.

Roder resolves the Python runtime in this order: `RODER_WEBWRIGHT_PYTHON`, `~/.roder/python/webwright/setup.json`, then system `python3`/`python`. `roder webwright run`, `roder webwright rerun`, and `webwright.run_script` all use this lookup when a Python override is not supplied.

The upstream skill prefers Firefox with `viewport={"width": 1280, "height": 1800}` and forbids `page.screenshot(full_page=True)`. Roder keeps the browser configurable, but Firefox is the documented default for Webwright mode.

## Security Model

Webwright workspaces stay under the current workspace by default, typically `.roder/webwright/<task-id>/`. Tools reject paths that escape the workspace root. Browser cookies, local storage, bearer tokens, raw headers, and credentials must not be copied into reports, logs, docs, tests, or exported task packages. Transcript-facing log tails, stdout/stderr, and verification messages redact common secret-bearing lines such as `Authorization: Bearer ...`, `token=...`, `password: ...`, and `api_key=...`.

## Verification

Deterministic verification runs offline and checks:

- Required workspace files and latest-run files exist.
- The critical-point checklist in `plan.md` has at least one item and every item is checked.
- The latest run has at least one `final_execution_*.png` screenshot.
- The latest `final_script_log.txt` contains a non-empty `final datum:` line.
- Root and run scripts do not request full-page screenshots.

Optional visual judging is intentionally disabled by default. It must use the current Roder inference provider, an image-capable model, and an explicit opt-in before any screenshot is sent to a model. Set `RODER_WEBWRIGHT_VISUAL_JUDGE=1` or pass `enabled: true` through the app-server method to opt in.

## Troubleshooting

- Missing Python: install Python 3.10+ and rerun `roder webwright setup --browser firefox`.
- Missing Playwright package: run `roder webwright setup --browser firefox`.
- Missing Firefox browser binaries: run `roder webwright setup --browser firefox`; use `--browser chromium` or `--browser webkit` when the task should target another Playwright browser.
- Verification fails on screenshots: save viewport screenshots under `final_runs/run_<id>/screenshots/final_execution_<step>_<label>.png`.
- Verification fails on final datum: add one clear `final datum: ...` line to the latest run log.
- Live browser checks: set `RODER_WEBWRIGHT_LIVE=1` and `RODER_WEBWRIGHT_START_URL=<url>` explicitly. Use `RODER_WEBWRIGHT_PYTHON=/path/to/python` to run from an isolated Playwright venv. Normal tests stay offline.
