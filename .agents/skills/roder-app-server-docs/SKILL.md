---
name: roder-app-server-docs
description: Maintain detailed API-integrator-level documentation for the Roder app-server API. Use when creating, updating, auditing, or reviewing docs for roder-app-server endpoints, JSON-RPC methods, session/thread APIs, provider/model APIs, auth/config behavior, streaming events, error shapes, or client integration guidance under docs/.
---

# Roder App-Server Docs

## Goal

Keep `docs/roder-app-server-api.md` accurate enough for an external or sibling-app integrator to build against the Roder app-server without reading the Rust source first.

## Workflow

1. Treat source code as the contract. Do not invent endpoint names, request fields, event names, defaults, or error behavior from memory.
2. Start from app-server entrypoints, route registration, request/response DTOs, and e2e tests before editing docs.
3. Update `docs/roder-app-server-api.md` for every app-server-facing API change, including new methods, changed fields, removed fields, event payloads, auth/config requirements, and compatibility notes.
4. Preserve unrelated dirty work. If the repo contains edits you do not recognize, ignore them unless they directly affect the API surface being documented.
5. Keep docs concrete: include method names, request JSON, response JSON, event examples, error behavior, and integration sequencing.
6. If a source file being edited is over 500 lines, prefer extracting reusable documentation-generation or schema logic instead of adding more monolithic logic. Do not split prose docs just to satisfy the line guideline.

## Source Discovery

Use targeted searches instead of broad tree walks. Useful starting points usually include:

- `crates/roder-app-server` for routes, handlers, server wiring, and e2e tests.
- `crates/roder-protocol` for shared API types and protocol-visible method shapes.
- `crates/roder-core` for runtime/session/thread behavior exposed through app-server methods.
- `crates/roder-ext-*` only when an app-server method exposes extension-owned behavior.
- Existing `docs/*.md` files for related public contracts and naming conventions.

Prefer `rg` searches for exact method names, route strings, DTO names, event names, and test names. Avoid full repo scans unless targeted searches fail.

## Documentation Standard

Read `references/api-doc-standard.md` before writing or auditing the API documentation. It defines the required sections, level of detail, examples, and update checklist for `docs/roder-app-server-api.md`.

## Update Rules

- Document stable public behavior, not private implementation details.
- Call out experimental or compatibility-sensitive behavior explicitly.
- Include environment variables, auth file locations, provider/model notation, and desktop bridge expectations when they affect app-server clients.
- Keep request/response examples valid JSON and use realistic placeholder values.
- Prefer one canonical example per method plus notes for optional fields or variants.
- When behavior is inferred from tests rather than explicit handler code, say that in the docs or leave a maintenance note in the PR/summary.

## Completion Criteria

Before saying the docs are complete, confirm at least these surfaces have been considered for the changed area:

- Route or JSON-RPC method registration.
- Request and response structs.
- Streaming or lifecycle events.
- Error and cancellation behavior.
- Session/thread persistence behavior.
- Auth, provider, model, and config requirements.
- Existing e2e tests that act as integration examples.
