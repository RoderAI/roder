# Roder Thread Goals

Roder goals are durable per-thread objectives shared by the runtime, app-server,
TUI, and model-facing goal tools.

## Slash Command

- `/goal` shows the current goal summary.
- `/goal <objective>` creates or replaces the current thread goal and resumes it
  as active.
- `/goal edit` pre-fills the composer with the current objective.
- `/goal edit <objective>` updates the objective.
- `/goal pause` pauses autonomous continuation.
- `/goal resume` resumes autonomous continuation.
- `/goal clear` removes the goal.

The TUI footer shows the current goal status and a compact objective preview.

## Runtime Behavior

Active goals are injected into model developer instructions for each turn. After
a normal turn completion, the runtime starts another turn automatically when the
goal is still active and no turn is already running for the thread.

Continuation stops when a goal is paused, blocked, budget-limited, usage-limited,
complete, or cleared. Interrupted or failed turns do not schedule goal
continuation.

## Model Tools

Models can inspect and update the same thread goal state through:

- `get_goal`
- `create_goal`
- `update_goal`

`update_goal` accepts only `complete` or `blocked`. User-controlled operations
such as pause, resume, budget changes, and clear stay on the app-server/TUI path.
