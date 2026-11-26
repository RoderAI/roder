# Roder Audit Fixes Implementation Plan

**Goal:** Close the seven reliability and correctness gaps identified in the Roder v0.4.10 audit.

**Architecture:** Keep changes at the existing boundaries for turn admission, JSONL indexing, provider HTTP, WebSocket output, transcript paging, and patch updates. Verify each fix through its normal entry point or a controlled local fixture.

**Tech stack:** Rust, Tokio, JSONL, SQLite, WebSocket, JSON-RPC.

## Owned Paths

| Lane | Paths |
| --- | --- |
| Durability | `crates/roder-core`, `crates/roder-ext-jsonl-thread-store` |
| Providers | `crates/roder-ext-runner-hosted-common`, `crates/roder-ext-mcp` |
| History | `crates/roder-tui`, app-server turn workspace resolution |
| Integration | App-server hosted gateway, `crates/roder-edit-core`, `crates/roder-tools`, changeset |

## Dependency Checks

Start from the freshly fetched Roder master. Reuse the existing bounded history endpoint, patch preservation mode, and executor disconnect cleanup. Run offline tests and the repository formatting and changeset gates.

## Tasks

- [x] Record the accepted prompt before the turn can be interrupted.
- [x] Serialize thread-list index updates and recover incomplete indexes.

Run: `cargo test -p roder-core --lib` and `cargo test -p roder-ext-jsonl-thread-store`.

Acceptance: Started turns retain their prompts after interruption. Concurrent creates remain visible through the thread list.

- [x] Sanitize hosted-runner response errors.
- [x] Recover expired stateful MCP sessions on the next operation.

Run: `cargo test -p roder-ext-runner-hosted-common -p roder-ext-mcp`.

Acceptance: Provider diagnostics keep credentials private. A stale session cannot poison later operations or replay a tool call.

- [x] Page TUI resume and older history through the bounded endpoint.
- [x] Resolve the workspace through thread metadata in turn start.

Run: `cargo test -p roder-tui` and the focused app-server thread tests.

Acceptance: The TUI can load recent and older items, preserve live updates, and resume threads on stores that lack paging. Turn start can resolve the workspace without loading a transcript.

- [x] Bound hosted outbound WebSocket queues and writes.
- [x] Preserve line endings through local and remote patch entry points.

Run: Focused hosted gateway tests, `cargo test -p roder-edit-core`, and the remote patch tests in `roder-tools`.

Acceptance: A stalled client cannot grow an unbounded backlog or silently lose an event on an open connection. Edits of CRLF files keep CRLF.

- [x] Review the combined diff, add the changeset, and run repository gates.

Run: `make fmt`, `make fmt-check`, and `python3 scripts/check-changesets.py --base origin/master`.

Acceptance: Focused suites and release gates pass on the combined branch.
