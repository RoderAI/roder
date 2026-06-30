---
roder-protocol: minor
roder-sdk-typescript: minor
roder-sdk-python: minor
roder-app-server: patch
roder: patch
roder-edit-tools: patch
---

# Dependency refresh and runner lifecycle method manifest

Register the `runners/pause`, `runners/resume`, `runners/detach`, and
`runners/rejoin` methods in the app-server method manifest and regenerate the
checked-in JSON schema and the TypeScript/Python generated client types so they
expose the runner lifecycle surface.

Refresh dependencies across the workspace and SDKs after validating each change:

- Rust: semver-compatible lockfile updates plus major bumps of `which`
  (6 -> 8), `tokio-tungstenite` (0.28 -> 0.29), `rcgen` (0.13 -> 0.14),
  `rusqlite` (0.38 -> 0.40), and `sqlx` (0.8 -> 0.9). Fixed a `time` 0.3.52
  deprecation (`format_description::parse` -> `parse_borrowed`). The
  `agent-client-protocol-schema` 1.x major is deferred because it renames the
  ACP type surface and needs a dedicated ACP-compliance migration.
- TypeScript/edit-tools: bump `@types/node` to v26.
- Python SDK: refresh the uv lock (anyio, pytest, pyright, idna).
