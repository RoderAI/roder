# Roder File-Backed Dynamic Context

Roder stores oversized dynamic context in local context artifacts and gives the model compact references it can inspect on demand. This keeps provider-visible tool results and compaction summaries bounded without discarding the full output.

## Configuration

File-backed dynamic context is enabled by default. Users can toggle it in the TUI from Ctrl+P → Settings → `File-backed dynamic context`.

The persisted config lives in `~/.roder/config.toml`:

```toml
[context]
file_backed_dynamic_context = true
```

Set it to `false` to fall back to inline truncation and compact summaries without writing new context artifacts. Environment overrides are also supported:

```sh
RODER_DISABLE_CONTEXT_ARTIFACTS=1 roder
RODER_FILE_BACKED_DYNAMIC_CONTEXT=false roder
```

## Storage

With the default JSONL store, context artifacts live inside each thread directory:

```text
$RODER_DATA_DIR/threads/<thread-id>/artifacts/<turn-id>/<artifact-id>.txt
$RODER_DATA_DIR/threads/<thread-id>/artifacts/<turn-id>/<artifact-id>.json
```

If `RODER_DATA_DIR` is not set, the default JSONL thread store uses `~/.roder/threads/<thread-id>/artifacts/...`. When the PostgreSQL session store is selected, artifacts are stored in PostgreSQL under the same tenant/thread scope as session rows instead. Runtime session stores can provide their own artifact backend; PostgreSQL mode must not silently fall back to `~/.roder/context-artifacts`.

Artifacts are addressed by id and thread. Model tools and app-server methods never require a filesystem path, and cross-thread reads are rejected.

## Artifact Kinds

Supported kinds are:

```text
tool_output
command_stdout
command_stderr
terminal_transcript
chat_history
compaction_source
context_provider_dump
```

Long tool output uses `tool_output`. Capped app-server `command/exec` stdout and stderr use `command_stdout` and `command_stderr`. Local compaction writes the pre-compaction conversation to `chat_history`.

## Model References

When output is stored externally, inline context includes a compact reference:

```text
[artifact: tool_output grep lines=842 bytes=49152 id=artifact-...]
Use read_artifact, grep_artifact, or tail_artifact with artifact_id "artifact-..." to inspect more.
```

The inline text also includes a bounded head/tail excerpt so the model can decide whether the artifact is worth reading.

## Tools

The runtime registers these model-facing tools:

- `read_artifact`: read a context artifact by id with `start_line` and `limit`.
- `grep_artifact`: search a context artifact by id with a literal `query`.
- `tail_artifact`: read the final lines of a context artifact by id.

Tools are scoped to the current `thread_id` in `ToolExecutionContext`.

## App-Server Methods

App and app-server clients can use:

```text
artifact/list
artifact/read
artifact/grep
artifact/tail
artifact/delete
```

All methods require `threadId`. Read, grep, tail, and delete also require `artifactId`. Results return descriptors without `storePath`; direct filesystem access stays internal to Roder.

`command/exec` now returns optional `stdoutArtifact` and `stderrArtifact` descriptors when output caps are reached. Streamed command output still publishes `command/exec/outputDelta` notifications, with `capReached` preserved.

## Cleanup

Artifacts are Roder-owned by default and can be removed through `artifact/delete`. For local cleanup during development, remove the data-dir subdirectory:

```sh
rm -rf "${RODER_DATA_DIR:-$HOME/.roder}/threads/<thread-id>/artifacts"
```

## Offline Evals

File-backed context fixtures live in:

```text
evals/fixtures/context/file-backed/
```

Run the local offline evaluator with:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals cargo run -p roder -- eval run evals/fixtures/context/file-backed --offline
```

The report is written to `file-backed-context-report.json` in the output directory and records artifact read/grep/tail counts, estimated inline tokens saved, and answer correctness for each fixture.
