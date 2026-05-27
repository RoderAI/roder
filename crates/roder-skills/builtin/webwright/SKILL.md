---
name: webwright
description: Drive a local Playwright browser through a Roder-owned Webwright workspace with plan, final script, screenshots, logs, and verification evidence.
metadata:
  short-description: Browser automation
exposure: direct_only
---

Use this skill for browser tasks that need durable evidence, repeatable Playwright scripts, screenshots, logs, or reusable CLI automation.

## Modes

- `/webwright:run <task>`: solve the literal task values supplied by the user.
- `/webwright:craft <task>`: create a parameterized Python CLI where concrete task values become defaults and `--help` documents how to reuse it.

## Workspace Contract

Create or use one Webwright workspace under `.roder/webwright/<task-id>/` unless the user gives a scoped output directory.

Required artifacts:

- `plan.md` with `# Critical Points` and one independently verifiable checklist item per user constraint.
- `final_script.py` at the workspace root.
- `final_runs/run_<id>/final_script.py` for each clean execution.
- `final_runs/run_<id>/screenshots/final_execution_<step>_<label>.png` for constraint-relevant evidence.
- `final_runs/run_<id>/final_script_log.txt` with one `step <n> action:` line per constraint-relevant action and the final datum at the end.

Use `viewport={"width": 1280, "height": 1800}` for Playwright screenshots. Do not use `page.screenshot(full_page=True)`.

## Roder Tools

Prefer Webwright helper tools for setup and checks:

- `webwright.prepare_workspace`
- `webwright.allocate_run`
- `webwright.lint_script`
- `webwright.run_script`
- `webwright.list_artifacts`
- `webwright.read_log_tail`
- `webwright.verify_run`
- `webwright.summarize_verification`

Use normal Roder file, edit, shell, media, and artifact tools for the actual code-as-action loop. Keep generated files inside the Webwright workspace.

If local Playwright dependencies are missing, ask the user to run `roder webwright setup --browser firefox` or the selected browser (`chromium` or `webkit`). `webwright.run_script` uses the managed setup runtime automatically when no `python` override is supplied.

## Verification

Only mark the task done when every critical point is checked and backed by a screenshot path or exact log line from the latest run. If evidence is ambiguous, fix the script and rerun in the next `run_<id>` directory.

For `/webwright:craft`, also verify:

- The reusable function is import-safe and does not launch a browser at import time.
- `python final_runs/run_<id>/final_script.py` succeeds with no arguments.
- `python final_runs/run_<id>/final_script.py --help` shows the user-facing argument contract.
