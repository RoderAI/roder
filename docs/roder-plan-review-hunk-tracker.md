# Roder Plan Review And Hunk Tracker

Plan review is the protocol-visible layer on top of plan mode. Plan mode still owns whether file-changing tools can run; plan review records the reviewable artifact, comments, rewrites, approvals, and resulting hunks so app-server clients and the TUI can display the work without inventing their own state.

## Events

- `plan/reviewCreated`: stores a `PlanReview` with stable `reviewId`, `threadId`, `turnId`, status, markdown, steps, comments, and rewrites.
- `plan/reviewStatusChanged`: moves a review through drafted, awaiting review, rewritten, approved, executing, completed, or rejected states.
- `plan/reviewCommentAdded`: records a user comment anchored to the whole plan, a step, a file and optional line range, or a hunk.
- `plan/reviewRewritten`: records a replacement plan body.
- `plan/reviewApproved` and `plan/reviewRejected`: record explicit review decisions.
- `hunk/recorded`: stores a `HunkRecord` linked to the producing tool call, optional plan review, optional plan step, and optional checkpoint id.
- `hunk/rollbackRequested` and `hunk/rollbackCompleted`: record rollback attempts and user-visible errors.

Diff bodies are pageable through `PagedHunkDiff`; hunk records retain capped inline diff lines for timeline display.

## App-Server Methods

- `plan/review/read`
- `plan/review/comment`
- `plan/review/rewrite`
- `plan/review/approve`
- `plan/review/reject`
- `hunk/list`
- `hunk/read`
- `hunk/rollback`

The app-server reconstructs review and hunk state from persisted session events. Comments and rewrites are appended as events so resumed clients see the same review history.

## Tool Reporting

`apply_patch`, `edit`, and `multi_edit` include `hunks` in their structured tool result data. The runtime converts those records into `hunk/recorded` events after the tool finishes. Apply-patch hunks use the Codex patch structure when available; edit tools report equivalent before/after line bodies from the exact replacement arguments.

## TUI

The timeline renders plan review rows and hunk rows inline. Review rows show status, steps, comment counts, and expanded markdown. Hunk rows show file path, rollback availability, and expanded diff lines. Keyboard navigation reuses the timeline focus model, so hunk and review rows are selectable without a side panel.

## Rollback

General checkpointing is deferred to [`../rfc/25-roder-checkpoint-and-undo.md`](../rfc/25-roder-checkpoint-and-undo.md). Hunk records can carry checkpoint ids and reverse patches, but app-server rollback currently returns a clear deferred-checkpoint error instead of mutating files. This keeps rollback visible to clients without reintroducing checkpoint implementation work.

## Verification

Use the normal phase tests for protocol, runtime, tools, app-server, and TUI surfaces. The optional visual smoke gate is:

```sh
RODER_TMUX_VISUAL=1 cargo test -p roder-tui-e2e plan_review -- --ignored
```

That package is optional; if it is not present in the workspace, record the skipped visual proof and rely on offline TUI render tests.
