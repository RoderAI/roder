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

- `webwright.prepare_workspace`: creates `plan.md`, `final_script.py`, and `webwright.json`. Preparing a workspace that already has a `webwright.json` keeps that manifest (its `latestRun`, `verificationState` and request fields) and says in the result text which requested fields were not applied; use a new `taskId` or `outputDir` to start over.
- `webwright.allocate_run`: creates the next `final_runs/run_<id>/` directory and copies the current final script.
- `webwright.lint_script`: rejects full-page screenshots and scripts without an import-safe `__main__` guard.
- `webwright.run_script`: executes the copied final script through the current process-runner policy gate, using the managed Webwright runtime when no `python` override is passed. The result text explains the run, see [Run Results](#run-results).
- `webwright.list_artifacts`: returns the structured workspace summary.
- `webwright.read_log_tail`: reads a redacted tail of the latest `final_script_log.txt`.
- `webwright.verify_run`: fails the tool call when deterministic verification fails. A run executed by `webwright.run_script` that did not exit 0 fails it even when its log and screenshots look complete; a run executed outside that tool has no recorded exit status, so verification cannot gate it on one.
- `webwright.summarize_verification`: returns the same verification state without marking the tool call failed.

These helpers do not replace normal Roder file, edit, shell, media, and artifact tools. They remove repetitive Webwright ceremony while keeping all paths scoped to the current workspace.

## Run Results

`webwright.run_script` puts what is needed to fix a failed run in the result text, because the transcript does not display the structured `data` (which still carries `outcome`, `exitCode`, `elapsedMs`, `hint`, `firstError`, `stdout` and `stderr` for clients). A passing run is one line:

```text
webwright run_001 ok: exit code 0 in 4.2s
```

A failing run starts with an outcome class and the elapsed time, then a hint, the first error line and the end of the output:

```text
webwright run_002 failed (nonzero_exit): exit code 1 after 2.3s
hint: timeout_error (guessed from the stderr text; trust the tail over this label)
first error (untrusted script output, secrets redacted): playwright._impl._errors.TimeoutError: Locator.click: Timeout 30000ms exceeded.
stderr tail (812 bytes; untrusted script output, secrets redacted):
...
next: fix final_script.py and call webwright.run_script again (every call allocates a new run); webwright.read_log_tail shows final_script_log.txt.
```

- The outcome class is decided by code from the process: `ok` (exit code 0), `nonzero_exit`, `signaled` (no exit code), `timeout` (killed at `timeoutSeconds`, default 60) or `launch_failed` (the interpreter could not be started; the text names it).
- The `hint` is a guess from well-known Python and Playwright message text (`missing_python_module`, `missing_browser_binary`, `python_syntax_error`, `navigation_failed`, `timeout_error`, `python_exception`). Those messages belong to other projects and change between versions, so the hint is a label only and the raw tail is always included.
- The first error line is the first unindented stderr line naming an `Error` or `Exception`, searched in the stderr the run kept (its first 256 KiB and last 768 KiB, see below) rather than only in the 4 KB tail. When stderr has no such line, the last non-empty stderr line is used; when stderr is empty there is no first error line and the stdout tail is shown instead.
- The tail is the last 4 KB of stderr, cut at a line start. When stderr is empty and the run failed, the last 2 KB of stdout is shown instead. Output is treated as untrusted page-derived text: secret-bearing lines are redacted, and both the first error line and the tail carry the label `untrusted script output, secrets redacted` themselves. Only the outcome class and the hint are decided by code.
- Each stream (stdout, stderr) keeps about 1 MiB at most: its first 256 KiB and its last 768 KiB, cut at line boundaries, with a `[... N bytes of script output omitted ...]` line where the middle was dropped. The structured `stdout` and `stderr` in `data`, and `final_script_log.txt` when the script wrote no log of its own, carry that bounded text, so a script that prints in a loop cannot exhaust the host's memory; the model text is still only the 4 KB (stderr) or 2 KB (stdout) tail.
- On a timeout the output written so far is kept. The script runs as the leader of its own process group on Unix, and the whole group is killed on timeout and also when the tool call is cancelled, so a run never outlives its tool call and neither do the helpers it started with `subprocess` or `cmd &`. Processes that moved themselves into another process group or session are not signalled and have to exit on their own when their parent goes away. A script that exits on its own is not followed: a background process it leaves behind keeps running. The script's stdin is closed.
- A background process the script started that still holds stdout or stderr open does not block the result for more than 2 seconds after the script exits.

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
      run_exit.json
      screenshots/
        final_execution_001_<label>.png
```

`run_exit.json` is written by `webwright.run_script` (`{"outcome", "exitCode", "elapsedMs"}`). It holds `running` while the script executes and is replaced when the run ends; verification reads it. Runs that were not executed by `webwright.run_script`, such as `webwright/rerun` or a script run by hand in a shell, have no `run_exit.json`.

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

Webwright workspaces stay under the current workspace by default, typically `.roder/webwright/<task-id>/`. Tools reject paths that escape the workspace root. Browser cookies, local storage, bearer tokens, raw headers, and credentials must not be copied into reports, logs, docs, tests, or exported task packages. Transcript-facing log tails, `webwright.run_script` result text, stdout/stderr, and verification messages redact common secret-bearing lines such as `Authorization: Bearer ...`, `token=...`, `password: ...`, and `api_key=...`.

## Verification

Deterministic verification runs offline and checks:

- Required workspace files and latest-run files exist.
- The critical-point checklist in `plan.md` has at least one item and every item is checked.
- The latest run has at least one `final_execution_*.png` screenshot.
- The latest `final_script_log.txt` contains a non-empty `final datum:` line.
- The latest run's recorded exit status (`script_exit`) is a clean exit. A nonzero exit, signal, timeout, launch failure, or a run recorded as still `running` (interrupted) fails verification, because a script can write a screenshot and a `final datum:` line and still fail afterwards. A run with no `run_exit.json` passes this check and the message says no status was recorded, so only runs executed by `webwright.run_script` are gated on their exit code.
- Root and run scripts do not request full-page screenshots.

Optional visual judging is intentionally disabled by default. It must use the current Roder inference provider, an image-capable model, and an explicit opt-in before any screenshot is sent to a model. Set `RODER_WEBWRIGHT_VISUAL_JUDGE=1` or pass `enabled: true` through the app-server method to opt in.

## Troubleshooting

- Missing Python: install Python 3.10+ and rerun `roder webwright setup --browser firefox`.
- Missing Playwright package: run `roder webwright setup --browser firefox`.
- Missing Firefox browser binaries: run `roder webwright setup --browser firefox`; use `--browser chromium` or `--browser webkit` when the task should target another Playwright browser.
- Verification fails on screenshots: save viewport screenshots under `final_runs/run_<id>/screenshots/final_execution_<step>_<label>.png`.
- Verification fails on final datum: add one clear `final datum: ...` line to the latest run log.
- Verification fails on `script_exit`: the latest run did not exit 0. Read the `webwright.run_script` result text for the first error and stderr tail, fix `final_script.py`, and run it again; each run gets a new `run_<id>` directory.
- `webwright.prepare_workspace` says it kept its existing `webwright.json`: the workspace already had a manifest, which is never overwritten. Pick a new `taskId` or `outputDir`, or delete `webwright.json`, to change the task, browser or start URL.
- Live browser checks: set `RODER_WEBWRIGHT_LIVE=1` and `RODER_WEBWRIGHT_START_URL=<url>` explicitly. Use `RODER_WEBWRIGHT_PYTHON=/path/to/python` to run from an isolated Playwright venv. Normal tests stay offline.
