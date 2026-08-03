## 0.1.1 (2026-08-03)

### Features

#### Add `/review`: a read-only review sub-turn with structured findings and pluggable review publishers

`/review` runs a detached, read-only reviewer over the working diff, a base
branch, a commit, or a free-form scope, and returns prioritized findings with
file/line locations. Findings render in a new TUI panel where they can be kept
or dropped, and are exposed over the app-server as `review/start`,
`review/publish`, and `review/publishers/list` plus `review/started`,
`review/completed`, `review/failed`, and `review/published` notifications.

Publishing goes through a new `ReviewPublisher` extension service. The
first-party `roder-ext-github-review` publisher submits findings as GitHub pull
request review comments over the `gh` CLI or the REST API, with diff-hunk
prefiltering and a dry-run mode. Configure it under `[review]` and
`[review.publishers.github]`. Each `[review.publishers.<id>]` block is stored
opaquely and parsed by the publisher's own crate, so adding a platform does not
change the core config types.

Also fixes `roder app-server`, which built its Tokio runtime in current-thread
mode. Providers that bridge a synchronous callback back into async work call
`tokio::task::block_in_place`, which panics outright on a current-thread
runtime, so the `claude-code` provider aborted the server on its first
Roder-executed tool call. The app-server now uses a multi-threaded runtime like
the TUI entry point.
