# Roder file-backed dynamic context

Long tool output, shell streams, terminal transcripts, and pre-compaction chat history are written to **context artifacts** under the Roder data directory. The model sees a compact inline summary plus an artifact reference it can follow with `read_artifact` / `grep_artifact` (and app-server `artifact/*` methods).

## Artifact kinds

| Kind | Source | Typical label |
|------|--------|----------------|
| `tool_output` | Capped tool results | `stdout` |
| `command_stdout` | App-server command output | `stdout` |
| `command_stderr` | App-server command stderr | `stderr` |
| `terminal_transcript` | Synced terminal sessions | `transcript` |
| `chat_history` | Conversation window before compaction | `chat_history` |
| `compaction_source` | Material used to build a summary | `source` |
| `context_provider_dump` | Large provider/context blocks | `dump` |

## Model-visible reference

```text
[artifact: tool_output stdout call_123 lines=842 path=turn-b/call_123_stdout.txt]
Use read_artifact or grep_artifact to inspect more.
```

Inline context also includes a short tail snippet when output was capped. The full payload is never dropped—only moved off-thread.

## Storage layout

Artifacts live **inside each session folder** next to `metadata.json` and `turn_items.jsonl`:

```text
~/.roder/sessions/{thread_id}/
  metadata.json
  turn_items.jsonl
  events.jsonl
  artifacts/
    metadata/{artifact_id}.json
    {turn_id}/{artifact_id}_{label}.txt
```

Override the sessions root with `RODER_SESSION_DIR` (default `~/.roder/sessions`). Tests may set `RuntimeConfig.context_artifact_dir` to pin a single store root.

Cross-thread reads are rejected in `ContextArtifactStore::get_for_thread`.

## Retention

Each artifact carries `ArtifactRetention`:

- `pinned: true` — skip automatic expiry (manual delete only)
- `expires_at` — optional RFC3339 timestamp; when past, runtime may emit `context/artifactRetentionExpired` and omit the artifact from new references

**Default today:** new artifacts are unpinned with no `expires_at` (kept until manual cleanup). A background sweeper is planned; until then, use the cleanup commands below.

Recommended policy for local machines:

| Kind | Suggested TTL | Notes |
|------|----------------|-------|
| `tool_output`, `command_stdout`, `command_stderr` | 7 days | High churn |
| `terminal_transcript` | 3 days | Large, repetitive |
| `chat_history`, `compaction_source` | 30 days | Needed for resumed threads |
| `context_provider_dump` | 7 days | Debug-oriented |

Pin artifacts referenced by active threads when implementing expiry.

## App-server API

```text
artifact/list
artifact/read
artifact/grep
artifact/tail
artifact/delete
```

All methods require the owning `thread_id` and artifact id (not arbitrary filesystem paths).

## Local cleanup

Inspect size:

```sh
du -sh "${RODER_SESSION_DIR:-$HOME/.roder/sessions}"/*/artifacts 2>/dev/null
find "${RODER_SESSION_DIR:-$HOME/.roder/sessions}" -path '*/artifacts/*/*.txt' | wc -l
```

Delete one artifact (metadata + data file via app-server or store API in tests):

```sh
# Example: remove metadata and body for a known id (destructive)
SESSION="${RODER_SESSION_DIR:-$HOME/.roder/sessions}/thread-a/artifacts"
rm -f "$SESSION/metadata/call_123.json"
rm -f "$SESSION/turn-b/call_123_stdout.txt"
```

Remove all artifacts for a session (destructive):

```sh
rm -rf "${RODER_SESSION_DIR:-$HOME/.roder/sessions}/thread-a/artifacts"
```

Nuke all session artifacts (destructive):

```sh
find "${RODER_SESSION_DIR:-$HOME/.roder/sessions}" -maxdepth 2 -type d -name artifacts -exec rm -rf {} +
```

Prefer `artifact/delete` from clients when integrated—filesystem deletes are for local dev recovery only.

## Evals

Fixtures: `evals/fixtures/context/file-backed/`

Metrics tracked by `roder-evals` graders:

- **inline_chars_saved** — `full_output_bytes - inline_context_bytes` (estimated token pressure reduction)
- **artifact_reads** — `read_artifact` / `artifact/read` calls in trajectory
- **artifact_grep_calls** — `grep_artifact` / `artifact/grep` calls
- **answer_correct** — normalized match against `expected_answer`

```sh
cargo test -p roder-evals file_backed_context
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals cargo run -p roder-cli -- eval run evals/fixtures/context/file-backed --offline
```

The offline CLI runner requires phase 44 (`roder eval run`); fixture JSON and graders are available now for harness integration.

## Related code

- `crates/roder-api/src/artifacts.rs` — types and reference formatting
- `crates/roder-core/src/artifacts.rs` — store, read/grep/tail/delete
- `crates/roder-core/src/tool_output.rs` — tool output capping
- `crates/roder-core/src/conversation.rs` — chat history before compaction
- `roadmap/53-roder-file-backed-dynamic-context.md` — implementation plan
