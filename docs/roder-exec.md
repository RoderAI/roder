# Roder Exec

`roder exec` is the non-TTY entrypoint for eval harnesses, scripts, and SDK
process transports. It uses the normal CLI config/auth path, starts the
in-process app-server, creates or resumes a thread, starts one turn, and waits
for the turn to complete.

```sh
printf 'Reply with exactly: ok\n' | roder exec --profile eval --mode bypass -
printf 'Reply with exactly: ok\n' | roder exec --json --profile eval --mode bypass -
roder exec resume --last 'continue'
roder exec resume THREAD_ID 'continue'
```

Default output is automation-safe:

- stdout: final assistant message only
- stderr: warnings, setup summaries, and diagnostics
- exit 0: turn completed
- exit 1: turn failed, was interrupted, or the runtime/app-server request failed

JSON mode emits one event per stdout line:

```json
{"type":"thread.started","thread_id":"..."}
{"type":"turn.started","turn_id":"..."}
{"type":"item.updated","item":{"id":"...","type":"agentMessage","text":"...","status":"inProgress"}}
{"type":"item.completed","item":{"id":"...","type":"agentMessage","status":"completed"}}
{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":92,"output_tokens":10,"reasoning_output_tokens":0,"cache_hit_rate":0.92}}
```

Supported options:

- `--json`: emit JSONL events to stdout
- `--output-last-message FILE`: write final assistant text to a file
- `--skip-git-repo-check`: accepted for benchmark sandboxes; Roder currently
  does not enforce a git repository check in this path
- `--ephemeral`: requests an ephemeral thread from the app-server
- `--profile eval|non_interactive`: selects an existing runtime profile
- `--mode bypass|accept_all|plan`: selects an existing policy mode
- `--image FILE`: attaches a local image input
- `--record-api-transcript FILE`: captures the app-server API transcript
  (header, requests, responses, notifications) as JSONL for debugging harness
  runs; the path is reported on stderr after the turn ends
- `-`: reads the prompt from stdin

For Harbor or Terminal-Bench runs, use an isolated config directory and keep
the JSONL stream as an artifact. Create artifact files before the model turn
starts so setup failures and timeouts still leave predictable paths for Harbor
to collect:

```sh
export RODER_CONFIG_DIR=/tmp/roder-harbor
export RODER_DATA_DIR=/tmp/roder-harbor
mkdir -p "$RODER_CONFIG_DIR/auth" /logs/agent
touch \
  /logs/agent/roder-cli.txt \
  /logs/agent/roder-events.jsonl \
  /logs/agent/roder-stderr.txt \
  /logs/agent/roder-last-message.txt \
  /logs/agent/setup-summary.txt
cp ~/.roder/auth/codex.json "$RODER_CONFIG_DIR/auth/codex.json"

printf '%s' "$TB_PROMPT" |
  roder exec --json --profile eval --mode bypass --skip-git-repo-check \
    --output-last-message /logs/agent/roder-last-message.txt - \
    >/logs/agent/roder-events.jsonl \
    2>/logs/agent/roder-stderr.txt
```

The deterministic Harbor artifact set is:

- `/logs/agent/roder-cli.txt`
- `/logs/agent/roder-events.jsonl`
- `/logs/agent/roder-stderr.txt`
- `/logs/agent/roder-last-message.txt`
- `/logs/agent/setup-summary.txt`

The Harbor adapter in `evals/harbor` follows this pattern and stores generated
jobs under `evals/harbor/jobs/`, which is gitignored. Analyze completed jobs
with `python3 evals/harbor/analyze_tbench_run.py JOB_DIR --require-clean` to
separate harness errors from reward-0 scored tasks.

To debug a failing harness task at the app-server API level, add
`--record-api-transcript /logs/agent/roder-api-transcript.jsonl` and replay or
inspect the captured JSON-RPC traffic.

A process-level offline smoke covers this contract end to end against the
fake provider (no TTY, no network):

```sh
cargo test -p roder --test exec_process_smoke
```
