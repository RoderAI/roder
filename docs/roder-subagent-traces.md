# Roder Subagent Traces

Roder subagent traces make task-scoped child work visible as first-class child turns instead of collapsed tool output. A parent turn that calls the `task` tool creates one `SubagentTraceSummary` per child and streams bounded `SubagentTraceDelta` items under that trace id.

## Terms

- `ParentTurnRef`: parent `thread_id` and `turn_id` that requested child work.
- `SubagentTraceId`: stable id for a child trace.
- `SubagentTraceStatus`: `queued`, `running`, `waiting_for_approval`, `completed`, `failed`, or `cancelled`.
- `SubagentDestination`: sanitized execution location. Current in-process subagents report `kind = in_process` and `label = in-process`; remote runner phases should use `remote_runner` with provider and destination ids, not secrets.
- `SubagentTraceSummary`: compact worker row data: task title, role, model, status, elapsed time, token usage, destination, latest activity, and optional error summary.
- `SubagentTraceDelta`: child message, reasoning, tool call, tool result, or status item. Tool result text is capped with pagination metadata.

## Events

Runtime and app-server clients receive these event kinds:

```text
turn/subagentTraceCreated
turn/subagentTraceDelta
turn/subagentTraceStatusChanged
turn/subagentTraceCompleted
turn/subagentTraceFailed
```

The event envelope uses the parent thread and turn ids so clients can subscribe to a parent turn and see all child workers. The payload also carries the trace id, so clients can filter one child trace locally.

## App-Server Methods

- `turn/subagentTraces/list` with `threadId` and `turnId` returns the current trace summaries for a parent turn.
- `turn/subagentTrace/read` with `threadId`, `traceId`, `offset`, and optional `limit` returns a bounded page of trace deltas plus `nextOffset` when more deltas exist.

These methods read from the thread event log. Live clients should subscribe to the event stream or JSON-RPC notifications and use read/list as resume or backfill paths.

## TUI Behavior

The TUI renders a compact worker row as soon as `turn/subagentTraceCreated` arrives. Rows show role, task, status, elapsed time, destination, and latest activity. `Enter` expands a selected worker inline to show child trace items. No side panel is used.

Controls:

- `j` / `k` and arrow keys move across visible timeline rows, including worker rows.
- `Tab` moves focus from composer to timeline; `Esc` returns focus to the composer.
- `Enter` expands or collapses the selected worker trace.
- Mouse click selects a worker row; a second click expands it.
- Mouse wheel and page keys preserve the existing timeline scroll behavior.

Auto-follow remains active while the user is at the bottom of the timeline. Manual scrolling disables auto-follow until the user returns to the end.

## Persistence

Trace events are appended to the thread event log, so resume restores parent/child trace relationships and final statuses. Child turn ids are stored in each summary for future deep-linking, while trace pages stay bounded for UI and protocol consumers.

## Visual References

The captured Grok Build media under `roadmap/assets/grok-build-2026-05-16/` is non-normative reference material. Roder keeps its own protocol names, event schema, and terminal layout conventions.
