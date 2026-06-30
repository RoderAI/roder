# Changelog

## 0.1.2 (2026-06-30)

### Fixes

#### Dependency refresh and runner lifecycle method manifest

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

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

## 0.0.0

- Initial public release for `@roderai/edit-tools`.
