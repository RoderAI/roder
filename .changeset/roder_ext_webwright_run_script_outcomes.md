---
roder-ext-webwright: minor
---

# webwright.run_script explains failed runs and gates verification on the exit code

- `webwright.run_script` reports an outcome class, the elapsed time, the first error line and a redacted stderr tail.
- The script is killed on timeout or cancellation, and each run writes `final_runs/run_<n>/run_exit.json`.
- `webwright.verify_run` gains a `script_exit` check, so a nonzero exit, a timeout or an interrupted run executed by `webwright.run_script` fails verification (a run with no recorded exit status, such as a script run by hand, is not judged by it).
- Re-preparing a workspace keeps its existing `webwright.json`.
