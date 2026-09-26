# Responses loop comparison

New evaluation runs default to **GPT-6-luna**. Live fixtures use `low` reasoning, the same instructions and tool allowlist, an isolated workspace and real filesystem assertions on both builds. These results compare the frozen Roder baseline (`816974be81b5cb7a1d249c14bc5bd26d7afb14ff`) with the local Responses parity implementation. Codex source reference: `8b78f4796605bda8e31329537f5fef036e405e66`.

## Reproduce

```sh
# Offline: 20 held-open terminal responses per build.
python3 docs/audits/responses-loop-eval/run.py

# Live: two editing tasks per build, HTTP held constant.
RODER_RESPONSES_PARITY_LIVE=1 python3 docs/audits/responses-loop-eval/run.py --live

# Current build WebSocket editing and native compaction smoke.
RODER_RESPONSES_PARITY_LIVE=1 python3 docs/audits/responses-loop-eval/run.py --live --current-only --transport websocket
RODER_RESPONSES_PARITY_LIVE=1 python3 docs/audits/responses-loop-eval/run.py --live --current-only --transport websocket --compaction-smoke
```

`--model` overrides Luna; `--scratch` retains the frozen source and result files. Baseline builds use an isolated Cargo target directory so their artifacts cannot contaminate current workspace checks. Live execution reads the Roder Codex login. The recorded run used the existing Codex CLI login via `RODER_PARITY_CODEX_AUTH_SOURCE=codex-cli` because the Roder login had expired. This option reads the saved access token directly for evaluation only; it does not refresh, copy or change either login. Credentials never appear in command arguments or result files.

## Evidence and limits

Checked-in `results/` files retain task checks, failures, durations, first-event timing, first tool start, patch errors, request byte/image metadata, tool counts and provider usage. First-event timing includes provider diagnostics and is not time to first token. The final Luna HTTP comparison passed both tasks on each build with no patch errors. Unicode update took 12,195 ms on the frozen baseline and 8,198 ms on current; move/add/delete took 12,850 ms and 13,544 ms respectively. Latency is mixed in this small sample. The compaction smoke passed, persisted an opaque boundary and answered BLUE in the subsequent turn. The small context estimate increased after compaction; this smoke verifies continuity, not token savings.

Live runs are sequential, unrandomized, one attempt per task. Cache state and service latency can differ. They do not establish a general speed or quality improvement, nor replace a full Terminal-Bench evaluation. The offline fixture holds the terminal SSE body open for 150 ms and isolates removal of the completion-to-EOF wait; its timing is not model throughput. Twenty runs passed on each build; median duration was 200.833 ms on baseline and 40.705 ms on current. `duration_ms` measures the first editing turn, including in the compaction smoke; subsequent compaction and follow-up are checked for continuity, not timed as a benchmark.

The active Harbor and OpenAI tool-search evaluation defaults also use Luna. Explicit older-model experiment configs and historical results retain their original model and reasoning settings.
